//! Presentation boundary: no ACP types, stdout writes or line-editor ownership.
use std::sync::Arc;

/// An adapter formats a complete Markdown message for a target viewport.
/// A full-screen UI may replace the presentation layer without changing ACP.
pub trait MessageRenderer: Send + Sync {
    fn render(&self, markdown: &str, width: usize) -> String;

    /// Render a streamed segment while carrying Markdown state across lines.
    /// Callers should commit the updated state only after the output is written.
    fn render_progressive(&self, markdown: &str, width: usize, _state: &mut RenderState) -> String {
        self.render(markdown, width)
    }
}

#[derive(Clone, Default)]
pub struct RenderState {
    fence: Option<(char, usize)>,
    bold: bool,
    inline_code: bool,
}

pub struct PlainRenderer;

impl MessageRenderer for PlainRenderer {
    fn render(&self, markdown: &str, _width: usize) -> String {
        sanitize(markdown)
    }
}

pub struct TermimadRenderer;

impl MessageRenderer for TermimadRenderer {
    fn render(&self, markdown: &str, width: usize) -> String {
        let clean = sanitize(markdown);
        let skin = termimad::MadSkin::default_dark();
        skin.text(&clean, Some(width.clamp(1, 4096))).to_string()
    }


    fn render_progressive(&self, markdown: &str, _width: usize, state: &mut RenderState) -> String {
        // Termimad needs a complete block for fences and tables. Keep its full
        // message renderer, but style streamed lines with a small state machine.
        let clean = sanitize(markdown);
        let mut output = String::with_capacity(clean.len() + 32);
        for segment in clean.split_inclusive('\n') {
            let (line, newline) = segment.strip_suffix('\n')
                .map_or((segment, ""), |line| (line, "\n"));
            if let Some((marker, count)) = state.fence {
                if is_closing_fence(line, marker, count) {
                    output.push_str("\x1b[2;38;5;245m");
                    output.push_str(line);
                    output.push_str("\x1b[0m");
                    state.fence = None;
                } else {
                    output.push_str("\x1b[38;5;252m");
                    output.push_str(line);
                    output.push_str("\x1b[0m");
                }
            } else if let Some(fence) = opening_fence(line) {
                state.fence = Some(fence);
                state.bold = false;
                state.inline_code = false;
                output.push_str("\x1b[2;38;5;245m");
                output.push_str(line);
                output.push_str("\x1b[0m");
            } else if line.trim_start().starts_with('|') {
                // A table may arrive before its separator. Preserve each row as
                // written instead of letting a single-row parser rearrange it.
                output.push_str("\x1b[38;5;250m");
                output.push_str(line);
                output.push_str("\x1b[0m");
            } else if let Some(heading) = heading_text(line) {
                output.push_str("\x1b[1;36m");
                output.push_str(heading);
                output.push_str("\x1b[0m");
            } else {
                render_inline(line, state, &mut output);
            }
            output.push_str(newline);
            if !newline.is_empty() {
                state.bold = false;
                state.inline_code = false;
            }
        }
        output
    }
}

fn opening_fence(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 { return None; }
    let marker = trimmed.chars().next()?;
    if marker != '`' && marker != '~' { return None; }
    let count = trimmed.chars().take_while(|value| *value == marker).count();
    (count >= 3).then_some((marker, count))
}

fn is_closing_fence(line: &str, marker: char, minimum: usize) -> bool {
    let trimmed = line.trim();
    let count = trimmed.chars().take_while(|value| *value == marker).count();
    count >= minimum && trimmed[count..].trim().is_empty()
}

fn heading_text(line: &str) -> Option<&str> {
    let count = line.chars().take_while(|value| *value == '#').count();
    (1..=6).contains(&count).then(|| line[count..].strip_prefix(' ')).flatten()
}

fn render_inline(line: &str, state: &mut RenderState, output: &mut String) {
    if state.inline_code {
        output.push_str("\x1b[38;5;214m");
    } else if state.bold {
        output.push_str("\x1b[1m");
    }
    let mut rest = line;
    while !rest.is_empty() {
        if rest.starts_with('`') {
            state.inline_code = !state.inline_code;
            output.push_str(if state.inline_code { "\x1b[38;5;214m" } else if state.bold { "\x1b[1m" } else { "\x1b[0m" });
            rest = &rest[1..];
        } else if !state.inline_code && rest.starts_with("**") {
            state.bold = !state.bold;
            output.push_str(if state.bold { "\x1b[1m" } else { "\x1b[0m" });
            rest = &rest[2..];
        } else {
            let character = rest.chars().next().expect("nonempty text");
            output.push(character);
            rest = &rest[character.len_utf8()..];
        }
    }
    if state.bold || state.inline_code { output.push_str("\x1b[0m"); }
}

pub fn renderer(styled: bool) -> Arc<dyn MessageRenderer> {
    if styled { Arc::new(TermimadRenderer) } else { Arc::new(PlainRenderer) }
}

pub fn terminal_width() -> usize {
    termimad::crossterm::terminal::size()
        .map(|(width, _)| usize::from(width).max(1))
        .unwrap_or(80)
}

/// Strip terminal-control sequences supplied by a model, not renderer styles.
/// Preserve Unicode and ordinary Markdown whitespace. OSC/DCS strings may
/// contain arbitrary payloads, so discard them through their terminator.
pub fn sanitize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => match chars.next() {
                Some('[') => { for c in chars.by_ref() { if ('@'..='~').contains(&c) { break; } } }
                Some(']') | Some('P') | Some('^') | Some('_') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' || c == '\u{9c}' { break; }
                        if c == '\u{1b}' && chars.peek() == Some(&'\\') { chars.next(); break; }
                    }
                }
                Some(c) if (' '..='/').contains(&c) => {
                    for c in chars.by_ref() { if ('0'..='~').contains(&c) { break; } }
                }
                _ => {}
            },
            '\r' => { if chars.peek() == Some(&'\n') { chars.next(); out.push('\n'); } }
            '\n' | '\t' => out.push(ch),
            ch if ch.is_control() => {},
            ch => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_output_preserves_markdown_and_unicode_without_styles() {
        let source = "# Título\n**rápido** y `Rust`\n你好\n";
        assert_eq!(renderer(false).render(source, 20), source);
    }

    #[test]
    fn strips_cursor_commands_and_osc_without_losing_text() {
        assert_eq!(sanitize("hola\x1b[2J\x1b]52;c;secret\x07mundo\r\n\x00!"), "holamundo\n!");
        assert_eq!(sanitize("a\x1b]0;title\x1b\\b"), "ab");
    }

    #[test]
    fn markdown_adapter_renders_complete_structures_at_narrow_width() {
        let source = "# Título\n\n**Hola**\n\n| A | B |\n|---|---|\n| uno | dos |\n\n```rust\nlet x = 1;\n```\n";
        for width in [1, 20, 80] {
            let rendered = renderer(true).render(source, width);
            assert!(!rendered.is_empty());
        }
        let rendered = sanitize(&renderer(true).render("**Hola**", 40));
        assert!(rendered.contains("Hola"));
        assert!(!rendered.contains("**"));
    }

    #[test]
    fn progressive_fence_keeps_code_styled_until_closing_line() {
        let renderer = TermimadRenderer;
        let mut state = RenderState::default();
        let open = renderer.render_progressive("```rust\n", 80, &mut state);
        let code = renderer.render_progressive("let café = 1;\n", 80, &mut state);
        let close = renderer.render_progressive("```\n", 80, &mut state);
        let prose = renderer.render_progressive("**Listo**\n", 80, &mut state);
        assert!(open.contains("```rust"));
        assert!(code.contains("let café = 1;"));
        assert!(code.starts_with("\x1b[38;5;252m"));
        assert!(close.contains("```"));
        assert!(prose.contains("Listo"));
        assert!(!prose.contains("**"));
        assert!(prose.ends_with('\n'));
    }

    #[test]
    fn progressive_table_rows_arrive_without_waiting_for_separator() {
        let renderer = TermimadRenderer;
        let mut state = RenderState::default();
        assert!(renderer.render_progressive("| A | B |\n", 80, &mut state).contains("| A | B |"));
        assert!(renderer.render_progressive("|---|---|\n", 80, &mut state).contains("|---|---|"));
    }

    #[test]
    fn plain_progressive_output_has_no_added_newline() {
        let renderer = PlainRenderer;
        let mut state = RenderState::default();
        assert_eq!(renderer.render_progressive("**rápido** 🚀", 80, &mut state), "**rápido** 🚀");
    }

    #[test]
    fn inline_style_continues_across_partial_chunks_and_resets_at_line_end() {
        let renderer = TermimadRenderer;
        let mut state = RenderState::default();
        let first = renderer.render_progressive("**muy ", 80, &mut state);
        let second = renderer.render_progressive("rápido**\n", 80, &mut state);
        let third = renderer.render_progressive("normal\n", 80, &mut state);
        assert!(first.contains("muy "));
        assert!(second.starts_with("\x1b[1mrápido"));
        assert_eq!(third, "normal\n");
    }
}

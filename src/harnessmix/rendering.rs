//! Presentation boundary: no ACP types, stdout writes or line-editor ownership.
use std::sync::Arc;

/// An adapter formats a complete Markdown message for a target viewport.
/// A full-screen UI may replace the presentation layer without changing ACP.
pub trait MessageRenderer: Send + Sync {
    fn render(&self, markdown: &str, width: usize) -> String;
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
}

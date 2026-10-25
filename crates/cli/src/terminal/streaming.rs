//! Incremental presentation framing, independent of the terminal and Markdown engine.
//! Every completed line is ready immediately, including lines inside tables and
//! fenced code. Segments concatenate back to the exact original input.

const SOFT_LINE_CHARS: usize = 64;
const MAX_SOFT_LINE_CHARS: usize = 128;
const MAX_PENDING_BYTES: usize = 64 * 1024;

#[derive(Default)]
pub struct StreamBuffer {
    pending: String,
    complete_lines_only: bool,
}

impl StreamBuffer {
    /// Use with a line editor that cannot safely display partial output lines.
    /// The unfinished final line remains available through `finish`.
    pub fn complete_lines() -> Self {
        Self { pending: String::new(), complete_lines_only: true }
    }

    pub fn push(&mut self, text: &str) -> Vec<String> {
        // In complete-line mode `pending` never contains a newline between
        // calls, so avoid rescanning a growing line for every small ACP delta.
        if self.complete_lines_only && !text.contains('\n') {
            self.pending.push_str(text);
            return Vec::new();
        }
        self.pending.push_str(text);
        let mut ready = Vec::new();
        loop {
            let boundary = self.boundary().or_else(|| {
                // A redirected stream may emit a long word without whitespace.
                // Interactive line editing intentionally waits for a newline.
                (!self.complete_lines_only && self.pending.len() > MAX_PENDING_BYTES).then(|| {
                    let mut end = MAX_PENDING_BYTES;
                    while !self.pending.is_char_boundary(end) { end -= 1; }
                    end
                })
            });
            let Some(end) = boundary else { break; };
            let tail = self.pending.split_off(end);
            ready.push(std::mem::replace(&mut self.pending, tail));
        }
        ready
    }

    /// Also used on cancellation/error: preserve an unfinished final block.
    pub fn finish(&mut self) -> Option<String> {
        if self.pending.is_empty() { None } else { Some(std::mem::take(&mut self.pending)) }
    }

    fn boundary(&self) -> Option<usize> {
        self.pending.find('\n').map(|index| index + 1).or_else(|| {
            (!self.complete_lines_only).then(|| self.soft_line_boundary()).flatten()
        })
    }

    fn soft_line_boundary(&self) -> Option<usize> {
        let mut hard_boundary = None;
        for (count, (index, character)) in self.pending.char_indices().enumerate() {
            if count >= SOFT_LINE_CHARS && character.is_whitespace() {
                return Some(index + character.len_utf8());
            }
            if count >= MAX_SOFT_LINE_CHARS {
                hard_boundary = Some(index);
                break;
            }
        }
        hard_boundary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_prose_is_available_before_the_turn_finishes() {
        let mut stream = StreamBuffer::default();
        assert!(stream.push("Primera res").is_empty());
        assert_eq!(stream.push("puesta.\nTodavía"), ["Primera respuesta.\n"]);
        assert_eq!(stream.finish().as_deref(), Some("Todavía"));
        assert!(stream.finish().is_none());
    }

    #[test]
    fn fences_split_across_chunks_are_emitted_line_by_line() {
        let mut stream = StreamBuffer::default();
        assert!(stream.push("``").is_empty());
        assert_eq!(stream.push("`rust\nfn main() {\n"), ["```rust\n", "fn main() {\n"]);
        assert_eq!(stream.push("}\n``"), ["}\n"]);
        assert_eq!(stream.push("`\nDespués\n"), ["```\n", "Después\n"]);
        assert!(stream.finish().is_none());
    }

    #[test]
    fn table_header_separator_and_rows_are_emitted_line_by_line() {
        let mut stream = StreamBuffer::default();
        assert_eq!(stream.push("| A | B |\n|-"), ["| A | B |\n"]);
        assert_eq!(stream.push("--|---|\n| 1 | 2 |\n"), ["|---|---|\n", "| 1 | 2 |\n"]);
        assert_eq!(stream.push("\nListo\n"), ["\n", "Listo\n"]);
    }

    #[test]
    fn unfinished_fence_is_retained_on_cancel_and_reset_for_next_turn() {
        let mut stream = StreamBuffer::default();
        assert_eq!(stream.push("~~~\nparcial"), ["~~~\n"]);
        assert_eq!(stream.finish().as_deref(), Some("parcial"));
        assert_eq!(stream.push("nuevo\n"), ["nuevo\n"]);
    }

    #[test]
    fn markdown_does_not_block_soft_progress_or_change_unicode() {
        let source = "**rápido** | `código` 🚀 ".repeat(20);
        let mut stream = StreamBuffer::default();
        let mut emitted = Vec::new();
        for character in source.chars() { emitted.extend(stream.push(&character.to_string())); }
        assert!(!emitted.is_empty());
        emitted.extend(stream.finish());
        assert_eq!(emitted.concat(), source);
    }

    #[test]
    fn interactive_mode_emits_only_complete_lines_until_finish() {
        let mut stream = StreamBuffer::complete_lines();
        let long = "**sin salto** 🚀 ".repeat(100);
        assert!(stream.push(&long).is_empty());
        assert_eq!(stream.push("\n"), [format!("{long}\n")]);
        assert_eq!(stream.finish(), None);
    }

    #[test]
    fn unicode_long_lines_make_progress_without_loss_or_repetition() {
        let source = "¡Hola 🌎! ".repeat(90);
        let mut stream = StreamBuffer::default();
        let mut emitted = Vec::new();
        for character in source.chars() { emitted.extend(stream.push(&character.to_string())); }
        assert!(!emitted.is_empty());
        emitted.extend(stream.finish());
        assert_eq!(emitted.concat(), source);
    }

    #[test]
    fn malformed_unbounded_block_has_a_bounded_retained_tail() {
        let source = format!("```\n{}", "é".repeat(MAX_PENDING_BYTES));
        let mut stream = StreamBuffer::default();
        let mut emitted = stream.push(&source);
        assert!(stream.pending.len() <= MAX_PENDING_BYTES);
        emitted.extend(stream.finish());
        assert_eq!(emitted.concat(), source);
    }
}

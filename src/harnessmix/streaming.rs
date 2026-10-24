//! Incremental presentation framing, independent of the terminal and Markdown engine.
//! Ordinary completed lines are ready immediately; tables and fenced code wait for
//! a structural boundary. Segments concatenate back to the exact original input.

const SOFT_LINE_CHARS: usize = 192;
const MAX_PENDING_BYTES: usize = 64 * 1024;

#[derive(Default)]
pub struct StreamBuffer {
    pending: String,
}

impl StreamBuffer {
    pub fn push(&mut self, text: &str) -> Vec<String> {
        self.pending.push_str(text);
        let mut ready = Vec::new();
        loop {
            let boundary = self.boundary().or_else(|| {
                // Pathological unclosed fences/tables/words cannot retain unlimited
                // memory. This fallback preserves bytes, at the cost of block styling.
                (self.pending.len() > MAX_PENDING_BYTES).then(|| {
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
        let Some(first_end) = self.pending.find('\n').map(|index| index + 1) else {
            return self.soft_line_boundary();
        };
        let first = &self.pending[..first_end];
        if let Some((marker, count)) = fence(first) {
            let mut offset = first_end;
            for line in self.pending[first_end..].split_inclusive('\n') {
                offset += line.len();
                if line.ends_with('\n') && closes_fence(line, marker, count) {
                    return Some(offset);
                }
            }
            return None;
        }
        if first.contains('|') {
            let mut offset = first_end;
            for line in self.pending[first_end..].split_inclusive('\n') {
                if !line.ends_with('\n') { return None; }
                if line.trim().is_empty() { return Some(offset + line.len()); }
                if !line.contains('|') { return Some(offset); }
                offset += line.len();
            }
            return None;
        }
        Some(first_end)
    }

    fn soft_line_boundary(&self) -> Option<usize> {
        // Never split a potential Markdown construct merely because it is long.
        // Complete lines/fences handle those; the hard cap is the safety valve.
        if self.pending.contains(['`', '~', '*', '_', '[', ']', '|', '\\']) {
            return None;
        }
        self.pending.char_indices().enumerate().find_map(|(count, (index, character))| {
            (count >= SOFT_LINE_CHARS && character.is_whitespace())
                .then_some(index + character.len_utf8())
        })
    }
}

fn fence(line: &str) -> Option<(char, usize)> {
    let line = line.trim_start_matches(' ');
    let marker = line.chars().next()?;
    if marker != '`' && marker != '~' { return None; }
    let count = line.chars().take_while(|value| *value == marker).count();
    (count >= 3).then_some((marker, count))
}

fn closes_fence(line: &str, marker: char, minimum: usize) -> bool {
    let line = line.trim();
    let count = line.chars().take_while(|value| *value == marker).count();
    count >= minimum && line[count..].trim().is_empty()
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
    fn fences_split_across_chunks_are_emitted_as_one_closed_block() {
        let mut stream = StreamBuffer::default();
        assert!(stream.push("``").is_empty());
        assert!(stream.push("`rust\nfn main() {\n").is_empty());
        assert!(stream.push("}\n``").is_empty());
        assert_eq!(stream.push("`\nDespués\n"), ["```rust\nfn main() {\n}\n```\n", "Después\n"]);
        assert!(stream.finish().is_none());
    }

    #[test]
    fn table_header_separator_and_rows_are_not_rendered_separately() {
        let mut stream = StreamBuffer::default();
        assert!(stream.push("| A | B |\n|-").is_empty());
        assert!(stream.push("--|---|\n| 1 | 2 |\n").is_empty());
        assert_eq!(stream.push("\nListo\n"), ["| A | B |\n|---|---|\n| 1 | 2 |\n\n", "Listo\n"]);
    }

    #[test]
    fn unfinished_fence_is_retained_on_cancel_and_reset_for_next_turn() {
        let mut stream = StreamBuffer::default();
        assert!(stream.push("~~~\nparcial").is_empty());
        assert_eq!(stream.finish().as_deref(), Some("~~~\nparcial"));
        assert_eq!(stream.push("nuevo\n"), ["nuevo\n"]);
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

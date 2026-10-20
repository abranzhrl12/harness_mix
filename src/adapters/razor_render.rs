//! Shared, bounded C# projection for Razor render expressions and statement blocks.
use harness_domain::{Failure, execution::OperationContext};

pub(super) struct Projection {
    pub text: String,
    pub line_offset: usize,
    shifts: Vec<(usize, usize, usize)>,
}
impl Projection {
    pub fn location(&self, line: usize, column: usize) -> Option<(usize, usize)> {
        let line = line
            .checked_sub(self.line_offset)
            .filter(|line| *line > 0)?;
        let index = self
            .shifts
            .partition_point(|&(row, start, _)| (row, start) <= (line, column));
        let shift = index
            .checked_sub(1)
            .and_then(|i| self.shifts.get(i))
            .is_some_and(|&(row, start, end)| row == line && column >= start && column < end);
        Some((line, column + usize::from(shift)))
    }
}
fn blank(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '\n' | '\r') {
            result.push(ch);
        } else {
            result.extend(std::iter::repeat_n(' ', ch.len_utf16()));
        }
    }
    result
}
pub(super) fn project(
    source: &str,
    fields: &str,
    context: &OperationContext,
) -> Result<Projection, Failure> {
    let mut blocks = Vec::new();
    super::csharp_syntax::collect_cshtml_blocks(source, "", context, &mut blocks, false)?;
    let mut spans = Vec::new();
    let mut at = 0;
    let bytes = source.as_bytes();
    let mut block = 0;
    while at < bytes.len() {
        context.check()?;
        while blocks.get(block).is_some_and(|(r, _)| r.end < at) {
            block += 1;
        }
        if let Some((r, _)) = blocks.get(block) {
            if at >= r.start.saturating_sub(1) && at <= r.end {
                at = r.end + 1;
                continue;
            }
        }
        if let Some(end) = super::csharp_syntax::razor_markup_end(bytes, at) {
            at = end;
            continue;
        }
        if bytes[at..].starts_with(b"@*") {
            at = source[at + 2..]
                .find("*@")
                .map_or(bytes.len(), |n| at + n + 4);
            continue;
        }
        if bytes[at..].starts_with(b"@@") {
            at += 2;
            continue;
        }
        if bytes[at..].starts_with(b"@{") {
            if let Some(end) = super::razor_syntax::block_end(bytes, at + 1) {
                spans.push((at + 2, end - 1, 0));
                at = end;
                continue;
            }
        }
        if bytes[at..].starts_with(b"@(") {
            if let Some(end) = super::razor_syntax::delimited_end(bytes, at + 1, b'(', b')') {
                spans.push((at + 2, end, 1));
                at = end;
                continue;
            }
        }
        if bytes[at] == b'@'
            && bytes
                .get(at + 1)
                .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        {
            let start = at;
            let mut end = at + 1;
            let mut called = false;
            loop {
                let word = end;
                while bytes
                    .get(end)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
                {
                    end += 1;
                }
                if end == word {
                    break;
                }
                if bytes.get(end) == Some(&b'(') {
                    let Some(close) = super::razor_syntax::delimited_end(bytes, end, b'(', b')')
                    else {
                        break;
                    };
                    end = close;
                    called = true;
                }
                if bytes.get(end) == Some(&b'.') {
                    end += 1;
                    continue;
                }
                break;
            }
            if called && !source[start..end].contains(['\n', '\r']) {
                spans.push((start, end, 2));
                at = end;
                continue;
            }
        }
        at += 1;
    }
    let mut member = String::new();
    let mut previous = 0;
    for (range, _) in &blocks {
        member.push_str(&blank(&source[previous..range.start]));
        member.push_str(&source[range.clone()]);
        previous = range.end;
    }
    member.push_str(&blank(&source[previous..]));
    let mut text =
        format!("class __HarnessRazor {{ {fields}\n{member}\nvoid __HarnessRender() {{\n");
    let line_offset = text.bytes().filter(|b| *b == b'\n').count();
    let mut shifts = Vec::new();
    previous = 0;
    let mut location_offset = 0;
    let mut line = 1;
    let mut column = 0;
    for (start, end, kind) in spans {
        // Spans are ordered. Advance one cursor rather than rescanning the
        // entire source prefix for every expression (quadratic on long views).
        for ch in source[location_offset..start].chars() {
            if ch == '\n' {
                line += 1;
                column = 0;
            } else {
                column += ch.len_utf16();
            }
        }
        text.push_str(&blank(&source[previous..start]));
        match kind {
            1 => {
                text.push_str(&source[start..end - 1]);
                text.push(';');
            }
            2 => {
                shifts.push((
                    line,
                    column,
                    column + source[start..end].encode_utf16().count(),
                ));
                text.push_str(&source[start + 1..end]);
                text.push(';');
            }
            _ => text.push_str(&source[start..end]),
        }
        for ch in source[start..end].chars() {
            if ch == '\n' {
                line += 1;
                column = 0;
            } else {
                column += ch.len_utf16();
            }
        }
        location_offset = end;
        previous = end;
    }
    text.push_str(&blank(&source[previous..]));
    text.push_str("\n}}\n");
    Ok(Projection {
        text,
        line_offset,
        shifts,
    })
}

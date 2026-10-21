use crate::Failure;
use std::borrow::Cow;
use std::collections::VecDeque;

pub(super) fn normalize(text: &str) -> Cow<'_, str> {
    if text.contains("\r\n") {
        Cow::Owned(text.replace("\r\n", "\n"))
    } else {
        Cow::Borrowed(text)
    }
}

/// Restrict matching to a block starting at the requested one-based line.
/// A line disambiguates repeated text, but never skips validating oldText.
pub(super) fn locate_chunk_at_line(
    hay: &str,
    needle: &str,
    line: usize,
) -> Result<(usize, usize), Failure> {
    let start = if line == 1 {
        0
    } else {
        hay.match_indices('\n')
            .nth(line - 2)
            .map(|(offset, _)| offset + 1)
            .filter(|offset| *offset < hay.len())
            .ok_or(("MATCH", "Requested line is outside the file"))?
    };
    let tail = &hay[start..];
    // Include exactly as many physical lines as oldText spans. Exact matching
    // remains unique inside this block, and the fallback retains indentation.
    let end = tail
        .match_indices('\n')
        .nth(needle.bytes().filter(|byte| *byte == b'\n').count())
        .map_or(hay.len(), |(offset, _)| start + offset);
    let (first, last) = locate_chunk(&hay[start..end], needle)?;
    if tail.find('\n').is_some_and(|newline| first > newline) {
        return Err(("MATCH", "Original text must start on the requested line"));
    }
    Ok((start + first, start + last))
}

pub(super) fn locate_chunk(hay: &str, needle: &str) -> Result<(usize, usize), Failure> {
    // 1. Try exact match first
    if let Some(first) = hay.find(needle) {
        let next = first + hay[first..].chars().next().unwrap_or(' ').len_utf8();
        if hay[next..].contains(needle) {
            return Err(("MATCH", "Original text must appear exactly once"));
        }
        return Ok((first, first + needle.len()));
    }

    // 2. Whitespace & indentation tolerant matching fallback. KMP over trimmed
    // lines preserves the original exact semantics without comparing every
    // possible file window against the entire oldText block.
    let needle_lines: Vec<&str> = needle.split('\n').collect();
    if needle_lines.is_empty() {
        return Err(("MATCH", "Original text must appear exactly once"));
    }
    let n = needle_lines.len();

    let mut prefix = vec![0usize; n];
    let mut matched = 0usize;
    for index in 1..n {
        while matched > 0 && needle_lines[index].trim() != needle_lines[matched].trim() {
            matched = prefix[matched - 1];
        }
        if needle_lines[index].trim() == needle_lines[matched].trim() {
            matched += 1;
        }
        prefix[index] = matched;
    }

    let mut starts = VecDeque::with_capacity(n);
    let mut line_start = 0usize;
    let mut matched = 0usize;
    let mut found = None;
    for line in hay.split('\n') {
        starts.push_back(line_start);
        if starts.len() > n {
            starts.pop_front();
        }
        while matched > 0 && line.trim() != needle_lines[matched].trim() {
            matched = prefix[matched - 1];
        }
        if line.trim() == needle_lines[matched].trim() {
            matched += 1;
        }
        if matched == n {
            let range = (starts[starts.len() - n], line_start + line.len());
            if found.is_some() {
                return Err(("MATCH", "Original text must appear exactly once"));
            }
            found = Some(range);
            matched = prefix[n - 1];
        }
        line_start += line.len() + 1;
    }

    found.ok_or(("MATCH", "Original text must appear exactly once"))
}

pub(super) enum Located {
    Missing,
    Ambiguous(usize),
    Unique,
}

pub(super) fn locate(hay: &str, needle: &str) -> Located {
    match hay.find(needle) {
        None => {
            // Check if it matches with whitespace tolerance
            if let Ok(_) = locate_chunk(hay, needle) {
                Located::Unique
            } else {
                Located::Missing
            }
        }
        Some(first) => {
            let next = first + hay[first..].chars().next().map_or(1, char::len_utf8);
            if hay[next..].contains(needle) {
                let mut occurrences = 1usize;
                let mut from = next;
                while let Some(position) = hay[from..].find(needle) {
                    occurrences += 1;
                    let absolute = from + position;
                    from = absolute + hay[absolute..].chars().next().map_or(1, char::len_utf8);
                    if occurrences >= 1000 {
                        break;
                    }
                }
                Located::Ambiguous(occurrences)
            } else {
                Located::Unique
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_fallback_returns_the_exact_unique_line_range() {
        let hay = "start\n  café 🙂\n  end\nfinish";
        let needle = "café 🙂\nend";
        assert_eq!(
            locate_chunk(hay, needle),
            Ok((6, 24)),
            "offsets remain UTF-8 byte offsets"
        );
    }

    #[test]
    fn whitespace_fallback_rejects_repeated_and_overlapping_blocks() {
        assert!(locate_chunk("a\nx\na\nx", "a\nx").is_err());
        assert!(locate_chunk("x\nx\nx", "x\nx").is_err());
    }

    #[test]
    fn crlf_normalization_keeps_ranges_in_the_normalized_source() {
        let hay = normalize("before\r\n  first\r\n  second\r\nafter");
        assert_eq!(locate_chunk(&hay, "first\nsecond"), Ok((7, 23)));
    }
}

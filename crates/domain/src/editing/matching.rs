use crate::Failure;
use std::borrow::Cow;

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

    // 2. Whitespace & indentation tolerant matching fallback:
    // Match line-by-line where each non-empty line has trimmed content matching.
    let needle_lines: Vec<&str> = needle.split('\n').collect();
    if needle_lines.is_empty() {
        return Err(("MATCH", "Original text must appear exactly once"));
    }

    // Build line slices with their byte offsets in hay
    let mut file_line_spans = Vec::new();
    let mut line_start = 0;
    for line in hay.split('\n') {
        let line_len = line.len();
        file_line_spans.push((line_start, line_start + line_len, line));
        line_start += line_len + 1; // + 1 for '\n'
    }

    let n = needle_lines.len();
    if file_line_spans.len() < n {
        return Err(("MATCH", "Original text must appear exactly once"));
    }

    let mut matches = Vec::new();
    for i in 0..=(file_line_spans.len() - n) {
        let mut all_match = true;
        for j in 0..n {
            let n_line = needle_lines[j].trim();
            let f_line = file_line_spans[i + j].2.trim();
            if n_line != f_line {
                all_match = false;
                break;
            }
        }
        if all_match {
            let match_start = file_line_spans[i].0;
            let match_end = file_line_spans[i + n - 1].1;
            matches.push((match_start, match_end));
            if matches.len() > 1 {
                return Err(("MATCH", "Original text must appear exactly once"));
            }
        }
    }

    if matches.len() == 1 {
        Ok(matches[0])
    } else {
        Err(("MATCH", "Original text must appear exactly once"))
    }
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

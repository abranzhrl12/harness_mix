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
/// A line disambiguates repeated text, not an unanchored search window.
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
    let needle_newlines = needle.bytes().filter(|byte| *byte == b'\n').count();
    // 1. Exact line count window (strict precedence)
    let end = tail
        .match_indices('\n')
        .nth(needle_newlines)
        .map_or(hay.len(), |(offset, _)| start + offset);
    if let Ok((first, last)) = locate_chunk(&hay[start..end], needle) {
        if tail.find('\n').is_none_or(|newline| first <= newline) {
            return Ok((start + first, start + last));
        }
    }
    // 2. Expanded tail window (+3 lines) so Stage 4 blank-line tolerance still works when anchored at `line`
    let expanded_end = tail
        .match_indices('\n')
        .nth(needle_newlines + 3)
        .map_or(hay.len(), |(offset, _)| start + offset);
    if expanded_end > end {
        if let Ok((first, last)) = locate_chunk(&hay[start..expanded_end], needle) {
            if tail.find('\n').is_none_or(|newline| first <= newline) {
                return Ok((start + first, start + last));
            }
        }
    }

    Err(("MATCH", "Original text must start on the requested line"))
}

fn fold_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for word in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// Normalize intra-line whitespace around punctuation/quotes and collapse spaces
/// so minor LLM formatting variations (e.g. `a = 1` vs `a=1`, `padding: 1px;` vs `padding:1px;`)
/// can be matched when unique.
fn canonicalize_tokens(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.trim().chars() {
        if ch.is_whitespace() {
            prev_space = true;
            continue;
        }
        let is_punct = matches!(
            ch,
            '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | ':' | '=' | '<' | '>' | '/' | '"' | '\''
        );
        if prev_space && !out.is_empty() {
            let last_punct = out.chars().last().is_some_and(|last| {
                matches!(
                    last,
                    '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | ':' | '=' | '<' | '>' | '/' | '"' | '\''
                )
            });
            if !is_punct && !last_punct {
                out.push(' ');
            }
        }
        out.push(ch);
        prev_space = false;
    }
    out
}

pub(super) fn locate_chunk(hay: &str, needle: &str) -> Result<(usize, usize), Failure> {
    // Stage 1: Exact substring match
    if let Some(first) = hay.find(needle) {
        let next = first + hay[first..].chars().next().unwrap_or(' ').len_utf8();
        if hay[next..].contains(needle) {
            return Err(("MATCH", "Original text must appear exactly once"));
        }
        return Ok((first, first + needle.len()));
    }

    let needle_lines: Vec<&str> = needle.split('\n').collect();
    if needle_lines.is_empty() {
        return Err(("MATCH", "Original text must appear exactly once"));
    }

    // Stage 2: Whitespace & indentation tolerant KMP over trimmed lines
    if let Some(range) = locate_kmp_lines(hay, &needle_lines, |line| line.trim())? {
        return Ok(range);
    }

    // Stage 3: Intra-line whitespace-folded KMP (collapses multiple spaces/tabs inside lines)
    if let Some(range) = locate_folded_lines(hay, &needle_lines, fold_whitespace)? {
        return Ok(range);
    }

    // Stage 4: Blank-line & punctuation-whitespace tolerant matching (requires >= 2 non-empty lines
    // or a single line of at least 16 non-space chars to prevent false positives on tiny tokens)
    let non_empty_needle: Vec<String> = needle_lines
        .iter()
        .map(|l| canonicalize_tokens(l))
        .filter(|l| !l.is_empty())
        .collect();
    let total_chars: usize = non_empty_needle.iter().map(String::len).sum();
    if (non_empty_needle.len() >= 2 && total_chars >= 12)
        || (non_empty_needle.len() == 1 && total_chars >= 20)
    {
        if let Some(range) = locate_non_empty_canonical(hay, &non_empty_needle)? {
            return Ok(range);
        }
    }

    Err(("MATCH", "Original text must appear exactly once"))
}

fn locate_kmp_lines<'a>(
    hay: &'a str,
    needle_lines: &[&'a str],
    transform: fn(&'a str) -> &'a str,
) -> Result<Option<(usize, usize)>, Failure> {
    let n = needle_lines.len();
    let needle_t: Vec<&str> = needle_lines.iter().map(|l| transform(l)).collect();
    if needle_t.iter().all(|l| l.is_empty()) {
        return Ok(None);
    }

    let mut prefix = vec![0usize; n];
    let mut matched = 0usize;
    for index in 1..n {
        while matched > 0 && needle_t[index] != needle_t[matched] {
            matched = prefix[matched - 1];
        }
        if needle_t[index] == needle_t[matched] {
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
        let line_t = transform(line);
        while matched > 0 && line_t != needle_t[matched] {
            matched = prefix[matched - 1];
        }
        if line_t == needle_t[matched] {
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

    Ok(found)
}

fn locate_folded_lines(
    hay: &str,
    needle_lines: &[&str],
    transform: fn(&str) -> String,
) -> Result<Option<(usize, usize)>, Failure> {
    let n = needle_lines.len();
    let needle_t: Vec<String> = needle_lines.iter().map(|l| transform(l)).collect();
    if needle_t.iter().all(String::is_empty) {
        return Ok(None);
    }

    let mut prefix = vec![0usize; n];
    let mut matched = 0usize;
    for index in 1..n {
        while matched > 0 && needle_t[index] != needle_t[matched] {
            matched = prefix[matched - 1];
        }
        if needle_t[index] == needle_t[matched] {
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
        let line_t = transform(line);
        while matched > 0 && line_t != needle_t[matched] {
            matched = prefix[matched - 1];
        }
        if line_t == needle_t[matched] {
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

    Ok(found)
}

/// Matches non-empty canonicalized lines across the file, tolerating extra or omitted
/// blank lines between matched lines (bounded to at most 3 consecutive blank lines).
fn locate_non_empty_canonical(
    hay: &str,
    needle_canon: &[String],
) -> Result<Option<(usize, usize)>, Failure> {
    let n = needle_canon.len();
    if n == 0 {
        return Ok(None);
    }

    let mut prefix = vec![0usize; n];
    let mut matched = 0usize;
    for index in 1..n {
        while matched > 0 && needle_canon[index] != needle_canon[matched] {
            matched = prefix[matched - 1];
        }
        if needle_canon[index] == needle_canon[matched] {
            matched += 1;
        }
        prefix[index] = matched;
    }

    let mut starts = VecDeque::with_capacity(n);
    let mut line_start = 0usize;
    let mut matched = 0usize;
    let mut blank_streak = 0usize;
    let mut found = None;

    for line in hay.split('\n') {
        let line_len = line.len();
        let canon = canonicalize_tokens(line);
        if canon.is_empty() {
            if matched > 0 {
                blank_streak += 1;
                if blank_streak > 3 {
                    matched = 0;
                    starts.clear();
                    blank_streak = 0;
                }
            }
            line_start += line_len + 1;
            continue;
        }
        blank_streak = 0;
        starts.push_back(line_start);
        if starts.len() > n {
            starts.pop_front();
        }
        while matched > 0 && canon != needle_canon[matched] {
            matched = prefix[matched - 1];
        }
        if canon == needle_canon[matched] {
            matched += 1;
        }
        if matched == n {
            let range = (starts[starts.len() - n], line_start + line_len);
            if found.is_some() {
                return Err(("MATCH", "Original text must appear exactly once"));
            }
            found = Some(range);
            matched = prefix[n - 1];
        }
        line_start += line_len + 1;
    }

    Ok(found)
}

pub(super) enum Located {
    Missing,
    Ambiguous(usize),
    Unique,
}

pub(super) fn locate(hay: &str, needle: &str) -> Located {
    match hay.find(needle) {
        None => Located::Missing,
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

    #[test]
    fn fusion_matching_tolerates_internal_whitespace_and_blank_lines() {
        let hay = "header\n  <button   type=\"button\"   class=\"btn\">\n\n    <span>OK</span>\n  </button>\nfooter";
        let needle = "<button type=\"button\" class=\"btn\">\n<span>OK</span>\n</button>";
        let (start, end) = locate_chunk(hay, needle).unwrap();
        assert_eq!(
            &hay[start..end],
            "  <button   type=\"button\"   class=\"btn\">\n\n    <span>OK</span>\n  </button>"
        );
    }

    #[test]
    fn locate_chunk_at_line_supports_fusion_and_blank_line_tolerance_on_target_line() {
        let hay = "line1\nline2\n  target_block_start()\n\n  target_block_end()\nline6\n";
        let needle = "target_block_start()\ntarget_block_end()";
        let (start, end) = locate_chunk_at_line(hay, needle, 3).unwrap();
        assert_eq!(
            &hay[start..end],
            "  target_block_start()\n\n  target_block_end()"
        );
    }
}

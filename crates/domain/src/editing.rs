//! Pure edit planning, compatible with Node's newline-tolerant unique matching.
use crate::{Failure, contracts::ReplaceArgs, hash};
use std::borrow::Cow;

pub const MAX_FILE: usize = 2 * 1024 * 1024;

#[derive(Default)]
struct OriginalOffsets {
    raw: usize,
    normalized: usize,
}

impl OriginalOffsets {
    // Requests must be monotonic; sorted, non-overlapping edit ranges guarantee it.
    fn at(&mut self, bytes: &[u8], offset: usize) -> usize {
        debug_assert!(offset >= self.normalized);
        while self.normalized < offset {
            self.raw += if bytes[self.raw..].starts_with(b"\r\n") {
                2
            } else {
                1
            };
            self.normalized += 1;
        }
        self.raw
    }
}

pub fn prepare(bytes: &[u8], args: &ReplaceArgs) -> Result<String, Failure> {
    let chunks: Vec<(&str, &str)> = match &args.chunks {
        Some(chunks) => {
            if !args.old_text.is_empty() || !args.new_text.is_empty() {
                return Err(("EDIT", "Use chunks or oldText/newText, not both"));
            }
            chunks
                .iter()
                .map(|c| (c.old_text.as_str(), c.new_text.as_str()))
                .collect()
        }
        None => vec![(&args.old_text, &args.new_text)],
    };
    if chunks.is_empty() || chunks.len() > 128 {
        return Err(("EDIT", "Expected between 1 and 128 replacement chunks"));
    }
    let mut budget = 0usize;
    for &(old, new) in &chunks {
        budget = budget.saturating_add(old.len()).saturating_add(new.len());
        if old.is_empty() || old == new || budget > 64000 {
            return Err(("EDIT", "Empty, identical or oversized replacement"));
        }
    }
    if !args.hash.is_empty()
        && (args.hash.len() != 64 || !args.hash.bytes().all(|b| b.is_ascii_hexdigit())) {
        return Err(("EDIT", "Expected a SHA-256 hash"));
    }
    if bytes.len() > MAX_FILE {
        return Err(("SIZE", "File exceeds 2 MiB"));
    }
    if !args.hash.is_empty() && hash(bytes) != args.hash {
        return Err(("STALE", "File changed since it was read"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ("ENCODING", "File is not UTF-8"))?;
    if text.contains('\0') {
        return Err(("ENCODING", "Binary files cannot be edited"));
    }
    let normalized = normalize(text);
    let mut edits = Vec::with_capacity(chunks.len());
    for (old, new) in chunks {
        let needle = normalize(old);
        let first = normalized
            .find(needle.as_ref())
            .ok_or(("MATCH", "Original text must appear exactly once"))?;
        let next = first + normalized[first..].chars().next().unwrap().len_utf8();
        if normalized[next..].contains(needle.as_ref()) {
            return Err(("MATCH", "Original text must appear exactly once"));
        }
        edits.push((first, first + needle.len(), Cow::Borrowed(new)));
    }
    edits.sort_unstable_by_key(|edit| edit.0);
    if edits.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err(("MATCH", "Replacement chunks overlap"));
    }
    let mut offsets = OriginalOffsets::default();
    for (start, end, new) in &mut edits {
        let (raw_start, raw_end) = if matches!(normalized, Cow::Borrowed(_)) {
            (*start, *end)
        } else {
            (
                offsets.at(text.as_bytes(), *start),
                offsets.at(text.as_bytes(), *end),
            )
        };
        let matched = &text[raw_start..raw_end];
        let eol = [matched, text]
            .into_iter()
            .find_map(|part| {
                part.find('\n').map(|i| {
                    if i > 0 && part.as_bytes()[i - 1] == b'\r' {
                        "\r\n"
                    } else {
                        "\n"
                    }
                })
            })
            .unwrap_or("\n");
        let replacement = normalize(new.as_ref()).replace('\n', eol);
        if replacement == matched {
            return Err(("EDIT", "Replacement does not change the file"));
        }
        *start = raw_start;
        *end = raw_end;
        *new = Cow::Owned(replacement);
    }
    let removed: usize = edits.iter().map(|(start, end, _)| end - start).sum();
    let added: usize = edits
        .iter()
        .map(|(_, _, replacement)| replacement.len())
        .sum();
    let size = text.len() - removed + added;
    if size > MAX_FILE {
        return Err(("SIZE", "Updated file exceeds 2 MiB"));
    }
    let mut updated = String::with_capacity(size);
    let mut cursor = 0;
    for (start, end, replacement) in edits {
        updated.push_str(&text[cursor..start]);
        updated.push_str(&replacement);
        cursor = end;
    }
    updated.push_str(&text[cursor..]);
    if updated == text {
        return Err(("EDIT", "Replacement does not change the file"));
    }
    if updated.contains('\0') {
        return Err(("EDIT", "Replacement contains NUL"));
    }
    Ok(updated)
}

fn normalize(text: &str) -> Cow<'_, str> {
    if text.contains("\r\n") {
        Cow::Owned(text.replace("\r\n", "\n"))
    } else {
        Cow::Borrowed(text)
    }
}

enum Located {
    Missing,
    Ambiguous(usize),
    Unique,
}

fn locate(hay: &str, needle: &str) -> Located {
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

fn excerpt(line: &str) -> String {
    const MAX: usize = 200;
    let trimmed = line.strip_suffix('\r').unwrap_or(line);
    if trimmed.chars().count() <= MAX {
        return trimmed.to_owned();
    }
    let mut out: String = trimmed.chars().take(MAX).collect();
    out.push('…');
    out
}

fn common_lines(needle: &[&str], file: &[&str]) -> usize {
    needle
        .iter()
        .zip(file)
        .take_while(|(expected, found)| expected == found)
        .count()
}

/// Best-effort, read-only explanation for a MATCH failure. Never mutates input and
/// never widens matching: it only reports where `oldText` stops matching the file.
pub fn match_hint(bytes: &[u8], args: &ReplaceArgs) -> serde_json::Value {
    use serde_json::json;
    let chunks: Vec<&str> = match &args.chunks {
        Some(chunks) => chunks.iter().map(|chunk| chunk.old_text.as_str()).collect(),
        None => vec![args.old_text.as_str()],
    };
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => return json!({}),
    };
    let normalized = normalize(text);
    for (index, old) in chunks.iter().enumerate() {
        if old.is_empty() {
            continue;
        }
        let needle = normalize(old);
        match locate(&normalized, needle.as_ref()) {
            Located::Unique => continue,
            Located::Ambiguous(occurrences) => {
                return json!({"chunk":index,"reason":"ambiguous","occurrences":occurrences,
                    "suggestion":"extend oldText so it matches exactly once"});
            }
            Located::Missing => return divergence(&normalized, needle.as_ref(), index),
        }
    }
    json!({})
}

fn divergence(hay: &str, needle: &str, chunk: usize) -> serde_json::Value {
    use serde_json::{Value, json};
    let needle_lines: Vec<&str> = needle.split('\n').collect();
    let file_lines: Vec<&str> = hay.split('\n').collect();
    let first = needle_lines[0];
    let mut best: Option<(usize, usize)> = None;
    let mut whitespace_only = false;
    for (index, line) in file_lines.iter().enumerate().take(20000) {
        if *line == first {
            let matched = common_lines(&needle_lines, &file_lines[index..]);
            if best.is_none_or(|(_, current)| matched > current) {
                best = Some((index, matched));
            }
        }
    }
    if best.is_none() && !first.trim().is_empty() {
        for (index, line) in file_lines.iter().enumerate().take(20000) {
            if line.trim() == first.trim() {
                best = Some((index, common_lines(&needle_lines, &file_lines[index..])));
                whitespace_only = true;
                break;
            }
        }
    }
    let mut hint = json!({"chunk":chunk});
    match best {
        Some((file_index, matched)) => {
            let divergent = matched.min(needle_lines.len().saturating_sub(1));
            hint["reason"] = json!("differing_line");
            hint["matchedLines"] = json!(matched);
            hint["needleLines"] = json!(needle_lines.len());
            hint["firstDivergentLine"] = json!(divergent + 1);
            hint["fileLine"] = json!(file_index + divergent + 1);
            hint["expectedLine"] = json!(excerpt(needle_lines[divergent]));
            hint["foundLine"] = file_lines
                .get(file_index + divergent)
                .map_or(Value::Null, |line| json!(excerpt(line)));
            if whitespace_only {
                hint["whitespaceOnlyMismatch"] = json!(true);
            }
        }
        None => {
            hint["reason"] = json!("first_line_not_found");
            hint["firstDivergentLine"] = json!(1);
            hint["needleLines"] = json!(needle_lines.len());
            hint["expectedLine"] = json!(excerpt(first));
        }
    }
    hint
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn monotonic_offsets_match_independent_normalized_prefixes() {
        for text in [
            "",
            "\r\n",
            "🙂\r\na\nñ\r\nb\r",
            "\r\n\r\nlast\r\n",
            "no newlines",
        ] {
            let normalized = normalize(text);
            let mut offsets = OriginalOffsets::default();
            for offset in 0..=normalized.len() {
                let expected = (0..=text.len())
                    .find(|&raw| {
                        // Only compare complete normalization boundaries, not half a CRLF.
                        !(raw > 0
                            && text.as_bytes().get(raw - 1) == Some(&b'\r')
                            && text.as_bytes().get(raw) == Some(&b'\n'))
                            && raw
                                - text.as_bytes()[..raw]
                                    .windows(2)
                                    .filter(|pair| *pair == b"\r\n")
                                    .count()
                                == offset
                    })
                    .unwrap();
                assert_eq!(
                    offsets.at(text.as_bytes(), offset),
                    expected,
                    "{text:?}, offset={offset}"
                );
            }
        }
    }
    fn multi(text: &str, chunks: &[(&str, &str)]) -> ReplaceArgs {
        let mut args = args(text, "", "");
        args.chunks = Some(
            chunks
                .iter()
                .map(|(old, new)| crate::contracts::ReplaceChunk {
                    old_text: (*old).into(),
                    new_text: (*new).into(),
                })
                .collect(),
        );
        args
    }

    #[test]
    fn match_hint_locates_first_diverging_line() {
        let text = "fn a() {}\nfn b() {}\nfn c() {}\n";
        let hint = match_hint(
            text.as_bytes(),
            &args(text, "fn a() {}\nfn B() {}\nfn c() {}", "x"),
        );
        assert_eq!(hint["reason"], "differing_line");
        assert_eq!(hint["firstDivergentLine"], 2);
        assert_eq!(hint["expectedLine"], "fn B() {}");
        assert_eq!(hint["foundLine"], "fn b() {}");
        assert_eq!(match_hint(b"x\nx", &args("x\nx", "x", "y"))["reason"], "ambiguous");
        assert_eq!(
            match_hint(text.as_bytes(), &args(text, "zzz", "y"))["reason"],
            "first_line_not_found"
        );
    }

    #[test]
    fn wire_modes_are_exclusive_and_round_trip_deletions() {
        for args in [args("abc", "a", ""), multi("abc", &[("a", "")])] {
            let value = serde_json::to_value(&args).unwrap();
            let decoded: ReplaceArgs = serde_json::from_value(value).unwrap();
            assert_eq!(prepare(b"abc", &decoded).unwrap(), "bc");
        }
        for fields in [
            serde_json::json!({"oldText":"a"}),
            serde_json::json!({"chunks":[],"oldText":"","newText":""}),
            serde_json::json!({"chunks":[{"oldText":"a"}]}),
        ] {
            let mut value = serde_json::json!({"path":"file.txt","hash":hash(b"abc")});
            value
                .as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            assert!(serde_json::from_value::<ReplaceArgs>(value).is_err());
        }
        assert_eq!(
            prepare(b"ab", &multi("ab", &[("a", "x"), ("b", "y")])).unwrap(),
            "xy"
        );
        assert_eq!(
            prepare(b"a", &multi("a", &vec![("a", "x"); 129]))
                .unwrap_err()
                .0,
            "EDIT"
        );
        let original = "a".repeat(MAX_FILE - 1) + "b";
        assert_eq!(
            prepare(original.as_bytes(), &multi(&original, &[("b", "bb")]))
                .unwrap_err()
                .0,
            "SIZE"
        );
    }

    #[test]
    fn multiple_chunks_use_original_offsets_and_preserve_eol() {
        let text = "\u{feff}🙂\r\nalpha\r\nbeta\r\ngamma\nend";
        let args = multi(text, &[("gamma", "G\nH"), ("alpha\nbeta", "A\nB")]);
        assert_eq!(
            prepare(text.as_bytes(), &args).unwrap(),
            "\u{feff}🙂\r\nA\r\nB\r\nG\r\nH\nend"
        );
        let args = multi("abc", &[("a", "b"), ("b", "a")]);
        assert_eq!(prepare(b"abc", &args).unwrap(), "bac");
    }

    #[test]
    fn multi_rejects_ambiguity_overlap_missing_and_mixed_modes() {
        for chunks in [
            vec![("aa", "x")],
            vec![("aaa", "x"), ("aa", "y")],
            vec![("missing", "x")],
            vec![("a", "x"), ("a", "y")],
        ] {
            assert_eq!(
                prepare(b"aaa", &multi("aaa", &chunks)).unwrap_err().0,
                "MATCH"
            );
        }
        assert_eq!(
            prepare(b"abc", &multi("abc", &[("ab", "x"), ("bc", "y")]))
                .unwrap_err()
                .0,
            "MATCH"
        );
        let mut args = multi("abc", &[("a", "x")]);
        args.old_text = "a".into();
        assert_eq!(prepare(b"abc", &args).unwrap_err().0, "EDIT");
        assert_eq!(prepare(b"abc", &multi("abc", &[])).unwrap_err().0, "EDIT");
        assert_eq!(
            prepare(b"abc", &multi("abc", &[("a", "\0")]))
                .unwrap_err()
                .0,
            "EDIT"
        );
        assert_eq!(
            prepare(
                b"abc",
                &multi("abc", &[("a", "x"), ("b", &"z".repeat(64000))])
            )
            .unwrap_err()
            .0,
            "EDIT"
        );
        assert_eq!(
            prepare(b"abd", &multi("abc", &[("a", "x")])).unwrap_err().0,
            "STALE"
        );
    }
    fn args(text: &str, old: &str, new: &str) -> ReplaceArgs {
        ReplaceArgs {
            path: "file.txt".into(),
            hash: hash(text.as_bytes()),
            chunks: None,
            old_text: old.into(),
            new_text: new.into(),
            allow_unvalidated_text: false,
            preview: false,
            session: None,
        }
    }
    #[test]
    fn preserves_bom_unicode_and_matched_newline_style() {
        let text = "\u{feff}🙂\nstart\r\nend\r\ntail";
        assert_eq!(
            prepare(text.as_bytes(), &args(text, "start\nend", "nuevo\nfin")).unwrap(),
            "\u{feff}🙂\nnuevo\r\nfin\r\ntail"
        );
    }
    #[test]
    fn rejects_overlap_and_newline_equivalent_duplicates() {
        for (text, old) in [("aaa", "aa"), ("🙂🙂🙂", "🙂🙂"), ("a\r\nb a\nb", "a\nb")] {
            assert_eq!(
                prepare(text.as_bytes(), &args(text, old, "x"))
                    .unwrap_err()
                    .0,
                "MATCH"
            );
        }
    }
    #[test]
    fn rejects_stale_noop_binary_and_oversized_edits() {
        let mut a = args("x", "x", "y");
        assert_eq!(prepare(b"z", &a).unwrap_err().0, "STALE");
        a.old_text.clear();
        assert_eq!(prepare(b"x", &a).unwrap_err().0, "EDIT");
        assert_eq!(
            prepare(b"x\0", &args("x\0", "x", "y")).unwrap_err().0,
            "ENCODING"
        );
        assert_eq!(
            prepare(b"a\r\nb", &args("a\r\nb", "a\nb", "a\r\nb"))
                .unwrap_err()
                .0,
            "EDIT"
        );
        assert_eq!(
            prepare(b"x", &args("x", "x", &"y".repeat(64000)))
                .unwrap_err()
                .0,
            "EDIT"
        );
    }
}

//! Pure edit planning, compatible with Node's newline-tolerant unique matching.
use crate::{Failure, contracts::ReplaceArgs, hash};
use std::borrow::Cow;

pub const MAX_FILE: usize = 2 * 1024 * 1024;

fn original_offset(bytes: &[u8], normalized_offset: usize) -> usize {
    let (mut raw, mut normalized) = (0, 0);
    while normalized < normalized_offset {
        raw += if bytes[raw..].starts_with(b"\r\n") {
            2
        } else {
            1
        };
        normalized += 1;
    }
    raw
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
    if args.hash.len() != 64 || !args.hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(("EDIT", "Expected a SHA-256 hash"));
    }
    if bytes.len() > MAX_FILE {
        return Err(("SIZE", "File exceeds 2 MiB"));
    }
    if hash(bytes) != args.hash {
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
        let (start, end) = if matches!(normalized, Cow::Borrowed(_)) {
            (first, first + needle.len())
        } else {
            (
                original_offset(text.as_bytes(), first),
                original_offset(text.as_bytes(), first + needle.len()),
            )
        };
        let matched = &text[start..end];
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
        let replacement = normalize(new).replace('\n', eol);
        if replacement == matched {
            return Err(("EDIT", "Replacement does not change the file"));
        }
        edits.push((start, end, replacement));
    }
    edits.sort_unstable_by_key(|edit| edit.0);
    if edits.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err(("MATCH", "Replacement chunks overlap"));
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

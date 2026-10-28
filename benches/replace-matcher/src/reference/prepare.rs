use super::encoding::{MAX_FILE, utf8_bom};
use super::matching::{locate_chunk, locate_chunk_at_line, normalize};
use crate::{Failure, contracts::ReplaceArgs, hash};
use std::borrow::Cow;

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
    if args.line == Some(0) || (args.line.is_some() && args.chunks.is_some()) {
        return Err((
            "EDIT",
            "line must be positive and used with oldText/newText, not chunks",
        ));
    }
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
        && (args.hash.len() != 64 || !args.hash.bytes().all(|b| b.is_ascii_hexdigit()))
    {
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
        let (first, end_offset) = match args.line {
            Some(line) => locate_chunk_at_line(&normalized, needle.as_ref(), line)?,
            None => locate_chunk(&normalized, needle.as_ref())?,
        };
        edits.push((first, end_offset, Cow::Borrowed(new)));
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
    utf8_bom(&args.path, &updated, args.bom, Some(bytes))
}

#[cfg(test)]
#[path = "prepare_tests.rs"]
mod tests;

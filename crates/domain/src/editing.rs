//! Pure edit planning, compatible with Node's newline-tolerant unique matching.
use crate::{Failure, contracts::ReplaceArgs, hash};

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
    if args.old_text.is_empty()
        || args.old_text == args.new_text
        || args.old_text.len().saturating_add(args.new_text.len()) > 64000
    {
        return Err(("EDIT", "Empty, identical or oversized replacement"));
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
    let normalized = text.replace("\r\n", "\n");
    let needle = args.old_text.replace("\r\n", "\n");
    let first = normalized
        .find(&needle)
        .ok_or(("MATCH", "Original text must appear exactly once"))?;
    let next = first + normalized[first..].chars().next().unwrap().len_utf8();
    if normalized[next..].contains(&needle) {
        return Err(("MATCH", "Original text must appear exactly once"));
    }
    let start = original_offset(text.as_bytes(), first);
    let end = original_offset(text.as_bytes(), first + needle.len());
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
    let replacement = args.new_text.replace("\r\n", "\n").replace('\n', eol);
    let size = text.len() - (end - start) + replacement.len();
    if size > MAX_FILE {
        return Err(("SIZE", "Updated file exceeds 2 MiB"));
    }
    let updated = format!("{}{}{}", &text[..start], replacement, &text[end..]);
    if updated == text {
        return Err(("EDIT", "Replacement does not change the file"));
    }
    if updated.contains('\0') {
        return Err(("EDIT", "Replacement contains NUL"));
    }
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(text: &str, old: &str, new: &str) -> ReplaceArgs {
        ReplaceArgs {
            path: "file.txt".into(),
            hash: hash(text.as_bytes()),
            old_text: old.into(),
            new_text: new.into(),
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

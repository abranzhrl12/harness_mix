use super::matching::{Located, locate, locate_chunk, locate_chunk_at_line, normalize};
use crate::contracts::ReplaceArgs;

const MAX_SEARCH_LINES: usize = 20_000;
const MAX_DIAGNOSTIC_CHUNKS: usize = 16;
const MAX_CANDIDATE_LINES: usize = 12;
const MAX_EXCERPT_CHARS: usize = 200;
const CONTEXT_LINES: usize = 2;

fn excerpt(line: &str) -> (String, bool) {
    let trimmed = line.strip_suffix('\r').unwrap_or(line);
    let mut chars = trimmed.chars();
    let text: String = chars.by_ref().take(MAX_EXCERPT_CHARS).collect();
    let truncated = chars.next().is_some();
    if truncated {
        (format!("{text}…"), true)
    } else {
        (text, false)
    }
}

fn common_lines<'a>(needle: &[&str], file: impl Iterator<Item = &'a str>) -> usize {
    needle
        .iter()
        .zip(file)
        .take_while(|(expected, found)| *expected == found)
        .count()
}

fn bounded_common_lines<'a>(needle: &[&str], file: impl Iterator<Item = &'a str>) -> usize {
    needle
        .iter()
        .zip(file)
        .take(MAX_CANDIDATE_LINES)
        .take_while(|(expected, found)| *expected == found)
        .count()
}

fn same_words(left: &str, right: &str) -> bool {
    left.split_whitespace().eq(right.split_whitespace())
}

fn bounded_common_word_lines<'a>(needle: &[&str], file: impl Iterator<Item = &'a str>) -> usize {
    needle
        .iter()
        .zip(file)
        .take(MAX_CANDIDATE_LINES)
        .take_while(|(expected, found)| same_words(expected, found))
        .count()
}

/// Best-effort, read-only explanation for a MATCH failure. It never mutates
/// input or broadens matching. All returned source context and diff rows are
/// bounded, and candidate search work is capped by lines rather than file size.
pub fn match_hint(bytes: &[u8], args: &ReplaceArgs) -> serde_json::Value {
    use serde_json::json;
    let chunks: Vec<&str> = match &args.chunks {
        Some(chunks) => chunks.iter().map(|chunk| chunk.old_text.as_str()).collect(),
        None => vec![args.old_text.as_str()],
    };
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => return json!({"truncated":false}),
    };
    let normalized = normalize(text);
    let mut first_match = None;
    for (index, old) in chunks.iter().take(MAX_DIAGNOSTIC_CHUNKS).enumerate() {
        if old.is_empty() {
            continue;
        }
        let needle = normalize(old);
        if let Some(line) = args.line
            && let Ok((offset, _)) = locate_chunk_at_line(&normalized, needle.as_ref(), line)
        {
            if first_match.is_none() {
                first_match = Some((index, offset, false));
            }
            continue;
        }
        match locate(&normalized, needle.as_ref()) {
            Located::Unique => {
                if first_match.is_none() {
                    if let Ok((offset, _)) = locate_chunk(&normalized, needle.as_ref()) {
                        first_match = Some((index, offset, args.line.is_some()));
                    }
                }
                continue;
            }
            Located::Ambiguous(occurrences) => {
                let mut hint = json!({"chunk":index,"reason":"ambiguous","occurrences":occurrences,
                    "suggestion":"extend oldText so it matches exactly once",
                    "truncated":occurrences >= 1000});
                if let Some((offset, _)) = normalized.match_indices(needle.as_ref()).next() {
                    let line = normalized[..offset]
                        .bytes()
                        .filter(|byte| *byte == b'\n')
                        .count()
                        + 1;
                    hint["fileLine"] = json!(line);
                    let context = source_context(text, line);
                    hint["actualSnippet"] = context.0;
                    hint["truncated"] = json!(hint["truncated"] == true || context.1);
                }
                return hint;
            }
            Located::Missing => return divergence(text, &normalized, needle.as_ref(), index),
        }
    }
    if chunks.len() > MAX_DIAGNOSTIC_CHUNKS {
        return json!({"reason":"diagnostic_chunk_limit","checkedChunks":MAX_DIAGNOSTIC_CHUNKS,
            "actualSnippet":first_match.map(|(_, offset, _)| {
                let line = normalized[..offset].bytes().filter(|byte| *byte == b'\n').count() + 1;
                source_context(text, line).0
            }).unwrap_or_else(|| json!([])),
            "truncated":true,
            "suggestion":"diagnostics inspected only the first 16 chunks; review remaining chunks with a fresh read"});
    }
    if let Some((chunk, offset, line_mismatch)) = first_match {
        let line = normalized[..offset]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1;
        let context = source_context(text, line);
        return json!({"chunk":chunk,"reason":if line_mismatch { "line_mismatch" } else { "text_matches_current_source" },"fileLine":line,
            "actualSnippet":context.0,"suggestion":if line_mismatch { "oldText still matches at a different line; verify the requested line before retrying" } else { "oldText still matches; verify the intended block before retrying with currentHash" },
            "truncated":context.1});
    }
    json!({"truncated":false})
}

fn source_context(text: &str, center_line: usize) -> (serde_json::Value, bool) {
    use serde_json::json;
    let start = center_line.saturating_sub(CONTEXT_LINES).max(1);
    let mut rows = Vec::new();
    let mut truncated = false;
    for (index, line) in text
        .split('\n')
        .enumerate()
        .skip(start - 1)
        .take(CONTEXT_LINES * 2 + 1)
    {
        let (text, clipped) = excerpt(line);
        truncated |= clipped;
        rows.push(json!({"line":index + 1,"text":text}));
    }
    (json!(rows), truncated)
}

fn divergence(source: &str, hay: &str, needle: &str, chunk: usize) -> serde_json::Value {
    use serde_json::{Value, json};
    let needle_lines: Vec<&str> = needle.split('\n').collect();
    let first = needle_lines[0];
    let mut best: Option<(usize, usize)> = None;
    let mut search_truncated = false;

    // Cloneable split iterators let each candidate inspect a short prefix without
    // re-walking the file from byte zero or allocating a vector of all file lines.
    let mut lines = hay.split('\n');
    for index in 0..MAX_SEARCH_LINES {
        let Some(line) = lines.next() else { break };
        if line == first {
            let matched = 1 + bounded_common_lines(&needle_lines[1..], lines.clone());
            if best.is_none_or(|(_, current)| matched > current) {
                best = Some((index, matched));
            }
        }
    }
    if lines.next().is_some() {
        search_truncated = true;
    }

    let mut whitespace_only = false;
    if best.is_none() && !first.trim().is_empty() {
        let mut lines = hay.split('\n');
        for index in 0..MAX_SEARCH_LINES {
            let Some(line) = lines.next() else { break };
            if line.trim() == first.trim() {
                let matched = 1 + bounded_common_lines(&needle_lines[1..], lines.clone());
                if best.is_none_or(|(_, current)| matched > current) {
                    best = Some((index, matched));
                }
            }
        }
        if lines.next().is_some() {
            search_truncated = true;
        }
        whitespace_only = best.is_some();
    }

    if best.is_none() && !first.trim().is_empty() {
        let mut lines = hay.split('\n');
        for index in 0..MAX_SEARCH_LINES {
            let Some(line) = lines.next() else { break };
            if same_words(line, first) {
                let matched = 1 + bounded_common_word_lines(&needle_lines[1..], lines.clone());
                if best.is_none_or(|(_, current)| matched > current) {
                    best = Some((index, matched));
                }
            }
        }
        if lines.next().is_some() {
            search_truncated = true;
        }
        whitespace_only = best.is_some();
    }

    let mut hint = json!({"chunk":chunk,"truncated":search_truncated});
    match best {
        Some((file_index, _)) => {
            // One unbounded comparison at the selected candidate is linear in the
            // already bounded request payload; candidate exploration above is fixed.
            let matched = common_lines(&needle_lines, hay.split('\n').skip(file_index));
            let divergent = matched.min(needle_lines.len().saturating_sub(1));
            let file_line = file_index + divergent + 1;
            hint["matchedLines"] = json!(matched);
            hint["needleLines"] = json!(needle_lines.len());
            hint["firstDivergentLine"] = json!(divergent + 1);
            hint["fileLine"] = json!(file_line);

            let (expected, expected_clipped) = excerpt(needle_lines[divergent]);
            let found_line = hay.split('\n').nth(file_index + divergent);
            let (found, found_clipped) = found_line.map_or((String::new(), false), excerpt);
            let whitespace_mismatch = whitespace_only
                || found_line.is_some_and(|line| same_words(needle_lines[divergent], line));
            hint["reason"] = json!(if whitespace_mismatch {
                "whitespace_mismatch"
            } else {
                "differing_line"
            });
            hint["expectedLine"] = json!(expected);
            hint["foundLine"] = found_line.map_or(Value::Null, |_| json!(found));

            let (context, context_clipped) = source_context(source, file_line);
            hint["actualSnippet"] = context;
            let diff = bounded_diff(
                &needle_lines,
                hay.split('\n').skip(file_index),
                divergent,
                file_index + 1,
            );
            hint["diffSuggestion"] = json!(diff.0);
            hint["truncated"] = json!(
                search_truncated || expected_clipped || found_clipped || context_clipped || diff.1
            );
        }
        None => {
            hint["reason"] = json!("first_line_not_found");
            hint["firstDivergentLine"] = json!(1);
            hint["needleLines"] = json!(needle_lines.len());
            let (expected, clipped) = excerpt(first);
            hint["expectedLine"] = json!(expected);
            hint["actualSnippet"] = json!([]);
            hint["diffSuggestion"] = json!([{"kind":"-","line":1,"text":expected}]);
            hint["truncated"] = json!(search_truncated || clipped || needle_lines.len() > 1);
        }
    }
    hint
}

fn bounded_diff<'a>(
    expected: &[&str],
    actual: impl Iterator<Item = &'a str>,
    divergent: usize,
    actual_start_line: usize,
) -> (serde_json::Value, bool) {
    use serde_json::json;
    let start = divergent.saturating_sub(CONTEXT_LINES);
    let expected_rows: Vec<_> = expected
        .iter()
        .skip(start)
        .take(CONTEXT_LINES * 2 + 1)
        .collect();
    let actual_rows: Vec<_> = actual.skip(start).take(CONTEXT_LINES * 2 + 1).collect();
    let mut rows = Vec::with_capacity(expected_rows.len() + actual_rows.len());
    let mut truncated = expected.len() > start + expected_rows.len();
    for (offset, line) in expected_rows.iter().enumerate() {
        let (text, clipped) = excerpt(line);
        truncated |= clipped;
        rows.push(json!({"kind":"-","line":start + offset + 1,"text":text}));
    }
    for (offset, line) in actual_rows.iter().enumerate() {
        let (text, clipped) = excerpt(line);
        truncated |= clipped;
        rows.push(json!({"kind":"+","line":actual_start_line + start + offset,"text":text}));
    }
    (json!(rows), truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::ReplaceChunk;

    fn args(old_text: &str) -> ReplaceArgs {
        ReplaceArgs {
            path: "sample.txt".into(),
            old_text: old_text.into(),
            new_text: "replacement".into(),
            line: None,
            chunks: None,
            hash: String::new(),
            bom: None,
            allow_unvalidated_text: false,
            preview: false,
            session: None,
        }
    }

    #[test]
    fn match_hint_reports_bounded_context_and_diff_for_crlf_unicode() {
        let source = "uno\r\n  café 🙂\r\nactual\r\nfin\r\n";
        let hint = match_hint(source.as_bytes(), &args("  café 🙂\nexpected\nfin"));
        assert_eq!(hint["reason"], "differing_line");
        assert_eq!(hint["fileLine"], 3);
        assert_eq!(hint["foundLine"], "actual");
        assert!(
            hint["actualSnippet"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["text"] == "  café 🙂")
        );
        assert!(
            hint["diffSuggestion"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["kind"] == "+" && row["text"] == "actual")
        );
        assert_eq!(hint["truncated"], false);
    }

    #[test]
    fn match_hint_identifies_whitespace_and_ambiguity() {
        let whitespace = match_hint(b"  value\n", &args("value\nother"));
        assert_eq!(whitespace["reason"], "whitespace_mismatch");
        let inner_whitespace = match_hint(b"first\nsecond  line\n", &args("first\nsecond line"));
        assert_eq!(inner_whitespace["reason"], "whitespace_mismatch");
        let ambiguous = match_hint(b"x\nx\n", &args("x"));
        assert_eq!(ambiguous["reason"], "ambiguous");
        assert_eq!(ambiguous["fileLine"], 1);
    }

    #[test]
    fn match_hint_caps_long_lines_and_search() {
        let long = format!("{}\n", "a".repeat(4096));
        let old = format!("{}\nmissing", "a".repeat(4096));
        let hint = match_hint(long.as_bytes(), &args(&old));
        assert_eq!(hint["truncated"], true);
        assert!(hint["expectedLine"].as_str().unwrap().chars().count() <= MAX_EXCERPT_CHARS + 1);

        let many_lines = format!("{}tail\n", "x\n".repeat(MAX_SEARCH_LINES + 2));
        let hint = match_hint(many_lines.as_bytes(), &args("not present"));
        assert_eq!(hint["truncated"], true);
    }

    #[test]
    fn chunks_report_the_first_failed_chunk_without_expanding_to_file_context() {
        let mut request = args("");
        request.chunks = Some(vec![
            ReplaceChunk {
                old_text: "first".into(),
                new_text: "one".into(),
            },
            ReplaceChunk {
                old_text: "missing".into(),
                new_text: "two".into(),
            },
        ]);
        let hint = match_hint(b"first\nother\n", &request);
        assert_eq!(hint["chunk"], 1);
        assert_eq!(hint["reason"], "first_line_not_found");
        assert!(hint.to_string().len() < 2000);
    }
}

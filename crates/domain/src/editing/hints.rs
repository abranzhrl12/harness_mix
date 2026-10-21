use super::matching::{Located, locate, normalize};
use crate::contracts::ReplaceArgs;

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

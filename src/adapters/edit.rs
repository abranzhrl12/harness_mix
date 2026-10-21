use super::{edit_commit, project::Project};
use harness_domain::{Failure, contracts::ReplaceArgs, editing, execution::OperationContext, hash};
use serde_json::{Value, json};
use std::path::Path;

pub(super) fn write(
    project: &Project,
    args: &harness_domain::contracts::WriteArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    super::create_file::write(project, args, context)
}

fn extension(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

pub(super) fn syntax(
    path: &str,
    source: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    match extension(path).as_str() {
        "json" => {
            serde_json::from_str::<Value>(source.strip_prefix('\u{feff}').unwrap_or(source))
                .map_err(|_| ("SYNTAX_ERROR", "Candidate JSON invalid; original untouched"))?;
            Ok(json!({"status":"valid","language":"json","validation":"syntax_only"}))
        }
        "md" | "txt" | "sql" | "xml" | "css" => {
            Ok(json!({"status":"not_applicable","language":"text","validation":"none"}))
        }
        _ => super::parsers::default_syntax_registry().validate(path, source, context),
    }
}

pub(super) fn syntax_allowing_unvalidated(
    path: &str,
    source: &str,
    allow_unvalidated_text: bool,
    context: &OperationContext,
) -> Result<Value, Failure> {
    match syntax(path, source, context) {
        Ok(value) => Ok(value),
        // Razor checks are lexical heuristics; an explicit opt-in must never block
        // the edit on a suspected false positive. Other parsers stay authoritative.
        Err((code, _)) if allow_unvalidated_text && extension(path) == "cshtml" => Ok(json!({
            "status":"not_checked","validation":"none",
            "reason":"explicitly_allowed","skippedSyntaxError":code,
            "explicitlyAllowed":true
        })),
        Err(("PARSER_UNAVAILABLE", _)) if allow_unvalidated_text => Ok(json!({
            "status":"not_checked","validation":"none",
            "reason":"parser_unavailable","explicitlyAllowed":true
        })),
        Err(other) => Err(other),
    }
}

/// Read-only MATCH diagnostic: locate where oldText stops matching the current file.
fn match_failure(path: &str, bytes: &[u8], args: &ReplaceArgs, message: &str) -> Value {
    json!({"ok":false,"path":path,"changed":false,
        "diagnostics":[{"code":"MATCH","message":message,"hint":editing::match_hint(bytes,args)}]})
}

pub(super) fn replace(
    project: &Project,
    args: &ReplaceArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    let target = edit_commit::target(project, &args.path)?;
    let _pins = edit_commit::pin_parents(project, &target)?;
    let bytes = project.bytes_with_context(&args.path, context)?;
    let updated = match editing::prepare(&bytes, args) {
        Ok(updated) => updated,
        Err(("MATCH", message)) => {
            return Ok(match_failure(&args.path, &bytes, args, message));
        }
        Err(failure) => return Err(failure),
    };
    let syntax =
        syntax_allowing_unvalidated(&args.path, &updated, args.allow_unvalidated_text, context)?;
    context.check()?;
    // Always recheck the source we actually prepared, including hashless requests.
    let mut bound = args.clone();
    bound.hash = hash(&bytes);
    let mut result = edit_commit::publish(project, &bound, &target, &updated, context)?;
    result["syntax"] = syntax;
    result["status"] = json!(if result["changed"] == true {
        "replaced"
    } else {
        "unchanged"
    });
    result["typecheck"] = json!("not_run");
    let before = std::str::from_utf8(&bytes).map_err(|_| ("ENCODING", "File must be UTF-8"))?;
    result["changeSummary"] = json!({"bytesBefore":bytes.len(),"bytesAfter":updated.len(),
        "linesBefore":before.lines().count(),"linesAfter":updated.lines().count(),
        "lineDelta":updated.lines().count() as i64 - before.lines().count() as i64});
    if result["diagnostics"].is_null() {
        result["diagnostics"] = json!([]);
    }
    if result["changed"] == true {
        // Commit has happened: cancellation must not hide this fact.
        let observed = project.bytes_with_context(&args.path, &OperationContext::default());
        let expected = hash(updated.as_bytes());
        let verified = observed.is_ok_and(|bytes| hash(&bytes) == expected);
        result["hash"] = json!(expected);
        result["verified"] = json!(verified);
        if !verified {
            result["ok"] = json!(false);
            result["diagnostics"] = json!([{"code":"EDIT_POSTCHECK","message":"Publication completed but postcheck failed; reread before retrying"}]);
        }
    }
    super::csproj::attach_warning(project, &args.path, &mut result, context);
    Ok(result)
}

/// Uses the same candidate preparation and parser as replace, without publication.
pub(super) fn preview(
    project: &Project,
    args: &ReplaceArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    edit_commit::target(project, &args.path)?;
    let bytes = project.bytes_with_context(&args.path, context)?;
    let updated = match editing::prepare(&bytes, args) {
        Ok(updated) => updated,
        Err(("MATCH", message)) => {
            return Ok(match_failure(&args.path, &bytes, args, message));
        }
        Err(failure) => return Err(failure),
    };
    let original = std::str::from_utf8(&bytes).map_err(|_| ("ENCODING", "File must be UTF-8"))?;
    let syntax = match syntax_allowing_unvalidated(
        &args.path,
        &updated,
        args.allow_unvalidated_text,
        context,
    ) {
        Ok(value) => value,
        Err((code, message)) => {
            json!({"status":"error","code":code,"message":message,"validation":"syntax_only"})
        }
    };
    context.check()?;
    let patch = preview_diff(&args.path, original, &updated);
    context.check()?;
    let truncated = patch.chars().count() > 12000;
    Ok(
        json!({"path":args.path,"baseHash":hash(&bytes),"candidateHash":hash(updated.as_bytes()),
        "changed":original!=updated,"published":false,"syntax":syntax,
        "patch":patch.chars().take(12000).collect::<String>(),"truncated":truncated,
        "diffStrategy":"bounded-line-alignment-with-three-context-lines",
        "notice":"Preview only. Apply the same replacement with harness_replace and baseHash; it rechecks the source. Syntax validation does not check types or compilation. A truncated patch is not applyable.",
        "_complete":!truncated,"_detail":{"patch":patch,"syntax":syntax}}),
    )
}

pub(super) fn preview_diff(path: &str, old: &str, new: &str) -> String {
    if old == new {
        return String::new();
    }
    // Keep newline bytes: CRLF and a missing final newline must remain observable.
    let a: Vec<_> = old.split_inclusive('\n').collect();
    let b: Vec<_> = new.split_inclusive('\n').collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let mut ops = Vec::new();
    ops.extend(a[..prefix].iter().map(|line| (' ', *line)));
    let (mut i, mut j) = (prefix, prefix);
    let (ae, be) = (a.len() - suffix, b.len() - suffix);
    while i < ae || j < be {
        if i < ae && j < be && a[i] == b[j] {
            ops.push((' ', a[i]));
            i += 1;
            j += 1;
            continue;
        }
        // Bound alignment work per change; fallback remains an exact, valid diff.
        let mut anchor = None;
        for x in i..(i + 64).min(ae) {
            for y in j..(j + 64).min(be) {
                if a[x] == b[y] && anchor.is_none_or(|(px, py)| x - i + y - j < px - i + py - j) {
                    anchor = Some((x, y));
                }
            }
        }
        let (x, y) = anchor.unwrap_or((ae, be));
        ops.extend(a[i..x].iter().map(|line| ('-', *line)));
        ops.extend(b[j..y].iter().map(|line| ('+', *line)));
        i = x;
        j = y;
    }
    ops.extend(a[ae..].iter().map(|line| (' ', *line)));
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for (index, _) in ops.iter().enumerate().filter(|(_, (kind, _))| *kind != ' ') {
        let start = index.saturating_sub(3);
        let end = (index + 4).min(ops.len());
        if let Some(last) = ranges.last_mut().filter(|last| last.1 >= start) {
            last.1 = end;
        } else {
            ranges.push((start, end));
        }
    }
    // JSON quoting prevents control characters in filenames forging patch headers.
    let mut out = format!(
        "--- {}\n+++ {}\n",
        json!(format!("a/{path}")),
        json!(format!("b/{path}"))
    );
    let (mut position, mut old_line, mut new_line) = (0, 0, 0);
    for (start, end) in ranges {
        for (kind, _) in &ops[position..start] {
            old_line += usize::from(*kind != '+');
            new_line += usize::from(*kind != '-');
        }
        let old_count = ops[start..end].iter().filter(|(k, _)| *k != '+').count();
        let new_count = ops[start..end].iter().filter(|(k, _)| *k != '-').count();
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            old_line + usize::from(old_count > 0),
            old_count,
            new_line + usize::from(new_count > 0),
            new_count
        ));
        for (kind, text) in &ops[start..end] {
            out.push(*kind);
            out.push_str(text);
            if !text.ends_with('\n') {
                out.push_str("\n\\ No newline at end of file\n");
            }
        }
        old_line += old_count;
        new_line += new_count;
        position = end;
    }
    out
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    #[test]
    fn diff_preserves_crlf_and_reports_missing_newline() {
        let patch = preview_diff("x.txt", "same\r\nold", "same\r\nnew\r\n");
        assert!(
            patch
                .contains("@@ -1,2 +1,2 @@\n same\r\n-old\n\\ No newline at end of file\n+new\r\n")
        );
        assert_eq!(preview_diff("x", "same", "same"), "");
        assert!(preview_diff("x", "", "new\n").contains("@@ -0,0 +1,1 @@"));
        assert!(preview_diff("x", "old\n", "").contains("@@ -1,1 +0,0 @@"));
    }

    #[test]
    fn distant_edits_have_separate_hunks_without_replacing_adjacent_code() {
        let old = (0..100).map(|i| format!("line {i}\n")).collect::<String>();
        let new = old
            .replace("line 1\n", "changed 1\n")
            .replace("line 90\n", "changed 90\n");
        let patch = preview_diff("x.txt", &old, &new);
        assert_eq!(patch.lines().filter(|l| l.starts_with("@@ ")).count(), 2);
        assert!(!patch.contains("line 50"));
        assert!(patch.contains("-line 1\n+changed 1\n"));
    }
}

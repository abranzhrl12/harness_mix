use super::project::{Failure, Project};
use harness_domain::{
    contracts::{Kind, SearchArgs},
    execution::OperationContext,
    results::{prefix, utf16_len},
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn excluded(name: &str) -> bool {
    let name = name.to_lowercase();
    [".git", ".hg", ".svn", "node_modules", "target"].contains(&name.as_str())
        || name.starts_with(".env")
        || name.starts_with("credential")
        || name.starts_with("secret")
        || name.starts_with("mcp_config.json")
        || ["oauth_creds.json", "id_rsa", "id_ed25519"].contains(&name.as_str())
}

#[cfg(test)]
pub fn search(project: &Project, args: &SearchArgs) -> Result<Value, Failure> {
    search_with_context(project, args, &OperationContext::default())
}
pub fn search_with_context(
    project: &Project,
    args: &SearchArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    if args.query.is_empty()
        || utf16_len(&args.query) > 16000
        || args.query.contains(['\n', '\r', '\0'])
    {
        return Err((
            "ARGUMENT",
            "query literal no vacía, sin saltos de línea ni NUL, máximo 16000 caracteres",
        ));
    }
    let limit = args.limit.unwrap_or(40);
    if !(1..=200).contains(&limit) {
        return Err(("ARGUMENT", "limit debe estar entre 1 y 200"));
    }
    let root = project.target(args.scope.as_deref().unwrap_or("."))?;
    if root
        .file_name()
        .is_some_and(|name| excluded(&name.to_string_lossy()))
    {
        return Err(("PATH", "scope excluido de búsquedas"));
    }
    let started = Instant::now();
    let mut builder = ignore::WalkBuilder::new(&root);
    builder
        .follow_links(false)
        .hidden(true)
        .parents(true)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(false)
        .require_git(false)
        .sort_by_file_path(|a, b| a.cmp(b))
        .filter_entry(|entry| !entry.file_name().to_str().is_none_or(excluded));
    let mut all = Vec::new();
    let mut visited = 0;
    let mut bytes = 0;
    let mut skipped = 0;
    let mut bounded = false;
    let mut timed_out = false;
    let mut cancelled = false;
    let mut truncated = false;
    'scan: for entry in builder.build() {
        if let Err((code, _)) = context.check() {
            cancelled = code == "CANCELLED";
            timed_out = code == "TIMEOUT";
            break;
        }
        if started.elapsed() >= Duration::from_secs(5) {
            timed_out = true;
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        if entry.error().is_some() {
            skipped += 1;
        }
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        visited += 1;
        if visited > 10000 {
            bounded = true;
            break;
        }
        let relative = match entry
            .path()
            .strip_prefix(&project.root)
            .ok()
            .and_then(|p| p.to_str())
        {
            Some(relative) => relative,
            None => {
                skipped += 1;
                continue;
            }
        };
        if project.target(relative).is_err() {
            skipped += 1;
            continue;
        }
        match args.kind {
            Kind::Files => {
                if relative.contains(&args.query) {
                    all.push(json!(relative));
                }
            }
            Kind::Text => {
                let file = match project.bytes_with_context(relative, context) {
                    Ok(file) => file,
                    Err((code, _)) if code == "CANCELLED" || code == "TIMEOUT" => {
                        cancelled = code == "CANCELLED";
                        timed_out = code == "TIMEOUT";
                        break 'scan;
                    }
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                };
                bytes += file.len();
                if bytes > 64 * 1024 * 1024 {
                    bounded = true;
                    break;
                }
                if file.contains(&0) {
                    continue;
                }
                let text = match std::str::from_utf8(&file) {
                    Ok(text) => text,
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                };
                for (line, text) in text.split_inclusive('\n').enumerate() {
                    if let Err((code, _)) = context.check() {
                        cancelled = code == "CANCELLED";
                        timed_out = code == "TIMEOUT";
                        break 'scan;
                    }
                    if started.elapsed() >= Duration::from_secs(5) {
                        timed_out = true;
                        break 'scan;
                    }
                    if text.contains(&args.query) {
                        truncated |= utf16_len(text) > 500;
                        all.push(json!({"file":relative,"line":line + 1,"text":text}));
                    }
                    // Bound stored results and a single file containing millions of lines.
                    if all.len() >= 10000 {
                        bounded = true;
                        break;
                    }
                }
            }
        }
        if all.len() >= 10000 {
            bounded = true;
            break;
        }
    }
    let complete = !bounded && !timed_out && !cancelled && skipped == 0;
    let mut compact: Vec<Value> = all.iter().take(limit).cloned().collect();
    if matches!(args.kind, Kind::Text) {
        for row in &mut compact {
            row["text"] = json!(prefix(row["text"].as_str().expect("match text"), 500).trim_end());
        }
    }
    let key = if matches!(args.kind, Kind::Files) {
        "paths"
    } else {
        "matches"
    };
    let mut result = json!({"omitted":all.len().saturating_sub(compact.len()),"truncated":truncated || !complete,"_complete":complete,
        "cancelled":cancelled,"timedOut":timed_out,"complete":complete,"scanned":visited,"bytesRead":bytes,"skipped":skipped,"limited":bounded,
        "timings":{"executionMs":started.elapsed().as_millis()},"_detail":{},"diagnostics":[]});
    result[key] = json!(compact);
    result["_detail"][key] = json!(all);
    if !complete {
        result["diagnostics"] = json!([{"code":"SEARCH_INCOMPLETE","message":"búsqueda parcial por presupuesto, plazo o archivos no accesibles; revisar complete/skipped/limited"}]);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn honors_ignore_secrets_and_compacts_without_losing_detail() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "ignored.txt\n").unwrap();
        for name in ["a.txt", "b.txt", "ignored.txt", "secret.txt", ".env"] {
            fs::write(dir.path().join(name), "needle 😀\n").unwrap();
        }
        let p = Project::new(dir.path()).unwrap();
        let args = SearchArgs {
            kind: Kind::Text,
            query: "needle".into(),
            scope: None,
            limit: Some(1),
            session: None,
        };
        let result = search(&p, &args).unwrap();
        assert_eq!(result["matches"].as_array().unwrap().len(), 1);
        assert_eq!(result["omitted"], 1);
        assert_eq!(result["_detail"]["matches"].as_array().unwrap().len(), 2);
        assert_eq!(result["complete"], true);
    }
    #[test]
    fn files_are_literal_and_unsafe_scope_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a[1].txt"), "text").unwrap();
        let p = Project::new(dir.path()).unwrap();
        let mut args = SearchArgs {
            kind: Kind::Files,
            query: "[1]".into(),
            scope: None,
            limit: None,
            session: None,
        };
        assert_eq!(
            search(&p, &args).unwrap()["paths"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        args.scope = Some("../outside".into());
        assert_eq!(search(&p, &args).unwrap_err().0, "PATH");
    }

    #[test]
    fn large_files_make_results_explicitly_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("large.txt"),
            vec![b'x'; 2 * 1024 * 1024 + 1],
        )
        .unwrap();
        fs::write(dir.path().join("small.txt"), "needle\n").unwrap();
        let p = Project::new(dir.path()).unwrap();
        let args = SearchArgs {
            kind: Kind::Text,
            query: "needle".into(),
            scope: None,
            limit: None,
            session: None,
        };
        let result = search(&p, &args).unwrap();
        assert_eq!(result["matches"].as_array().unwrap().len(), 1);
        assert_eq!(result["complete"], false);
        assert_eq!(result["_complete"], false);
        assert_eq!(result["skipped"], 1);
        assert_eq!(result["diagnostics"][0]["code"], "SEARCH_INCOMPLETE");
    }
}

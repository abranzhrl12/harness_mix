use super::project::{Failure, Project};
use harness_domain::{
    contracts::{Kind, SearchArgs},
    execution::OperationContext,
    results::{prefix, utf16_len},
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use std::{cell::RefCell, collections::BTreeMap};

struct FolderTiming<'a> {
    stats: &'a RefCell<BTreeMap<String, (u64, usize, usize)>>,
    folder: String,
    started: Instant,
    bytes: usize,
}
impl Drop for FolderTiming<'_> {
    fn drop(&mut self) {
        let mut stats = self.stats.borrow_mut();
        let folder = if stats.len() >= 128 && !stats.contains_key(&self.folder) {
            "[other folders]".to_owned()
        } else {
            self.folder.clone()
        };
        let entry = stats.entry(folder).or_default();
        entry.0 += self.started.elapsed().as_micros().min(u64::MAX as u128) as u64;
        entry.1 += 1;
        entry.2 += self.bytes;
    }
}

// A deliberately small declaration selector, not ast-grep pattern syntax.
fn selector(query: &str) -> Result<(&str, Option<&str>), Failure> {
    let (kind, name) = query
        .split_once(':')
        .map_or((query, None), |(k, n)| (k, Some(n)));
    if ![
        "function_item",
        "function_signature_item",
        "struct_item",
        "enum_item",
        "union_item",
        "trait_item",
        "mod_item",
        "type_item",
        "const_item",
        "static_item",
        "macro_definition",
        "impl_item",
    ]
    .contains(&kind)
        || name.is_some_and(|n| n.is_empty() || n.contains(':') || n.chars().any(char::is_control))
    {
        return Err((
            "ARGUMENT",
            "Rust syntax selector requires declaration_kind[:exact name]; AST patterns are not supported",
        ));
    }
    Ok((kind, name))
}

fn glob_matcher(
    globs: &Option<Vec<String>>,
    kind: &Kind,
) -> Result<Option<globset::GlobSet>, Failure> {
    let patterns = match globs {
        Some(patterns) => patterns,
        None => return Ok(None),
    };
    if !matches!(kind, Kind::Files | Kind::FileSize) {
        return Err((
            "ARGUMENT",
            "globs solo es válido para kind files o file_size",
        ));
    }
    if patterns.is_empty() || patterns.len() > 64 {
        return Err(("ARGUMENT", "globs requiere entre 1 y 64 patrones"));
    }
    let mut builder = globset::GlobSetBuilder::new();
    for pattern in patterns {
        if pattern.is_empty()
            || utf16_len(pattern) > 1024
            || pattern.contains(['\0', '\n', '\r'])
            || pattern.chars().any(char::is_control)
        {
            return Err((
                "ARGUMENT",
                "cada patrón glob debe ser no vacío, sin control/NUL ni saltos de línea, máximo 1024 caracteres",
            ));
        }
        let glob = globset::GlobBuilder::new(pattern)
            .literal_separator(false)
            .build()
            .map_err(|_| ("ARGUMENT", "patrón glob inválido"))?;
        builder.add(glob);
    }
    let set = builder
        .build()
        .map_err(|_| ("ARGUMENT", "patrón glob inválido"))?;
    Ok(Some(set))
}

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
    let file_size = matches!(args.kind, Kind::FileSize);
    let syntax = matches!(args.kind, Kind::Syntax);
    let selected = if syntax {
        Some(selector(&args.query)?)
    } else {
        None
    };
    let globs = glob_matcher(&args.globs, &args.kind)?;
    let min_lines = args.min_lines.unwrap_or(500);
    if min_lines > 10000000
        || (!file_size && args.min_lines.is_some())
        || (file_size && !args.query.is_empty())
    {
        return Err((
            "ARGUMENT",
            "file_size requires minLines 0..10000000 and no query; minLines is invalid for other kinds",
        ));
    }
    if !file_size
        && (args.query.is_empty()
            || utf16_len(&args.query) > 16000
            || args.query.contains(['\n', '\r', '\0']))
    {
        return Err((
            "ARGUMENT",
            "query literal no vacía, sin saltos de línea ni NUL, máximo 16000 caracteres",
        ));
    }
    let limit = args.limit.unwrap_or(if file_size { 30 } else { 40 });
    if !(1..=if file_size { 100 } else { 200 }).contains(&limit) {
        return Err((
            "ARGUMENT",
            if file_size {
                "file_size limit debe estar entre 1 y 100"
            } else {
                "limit debe estar entre 1 y 200"
            },
        ));
    }
    let root = project.target(args.scope.as_deref().unwrap_or("."))?;
    if root
        .file_name()
        .is_some_and(|name| excluded(&name.to_string_lossy()))
    {
        return Err(("PATH", "scope excluido de búsquedas"));
    }
    let started = Instant::now();
    let folder_stats = RefCell::new(BTreeMap::new());
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
        .filter_entry(move |entry| {
            !entry.file_name().to_str().is_none_or(|name| {
                excluded(name)
                    || (file_size
                        && (["vendor", "dist", "build", "coverage"]
                            .contains(&name.to_ascii_lowercase().as_str())
                            || name.ends_with(".min.js")
                            || name.ends_with(".map")))
            })
        });
    let mut all = Vec::new();
    let mut visited = 0;
    let mut bytes = 0;
    let mut reserved_bytes = 0_u64;
    let mut skipped = 0;
    let mut binary_skipped = 0;
    let mut parse_errors = 0;
    let mut stable_files = 0;
    let mut metadata_bytes = 0;
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
        let normalized = relative.replace('\\', "/");
        if let Some(set) = &globs {
            if !set.is_match(&normalized) {
                continue;
            }
        }
        let folder = normalized
            .rsplit_once('/')
            .map(|(parent, _)| parent.split('/').take(2).collect::<Vec<_>>().join("/"))
            .unwrap_or_else(|| ".".into());
        let mut file_timing = FolderTiming {
            stats: &folder_stats,
            folder,
            started: Instant::now(),
            bytes: 0,
        };
        if syntax
            && !entry
                .path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
        {
            continue;
        }
        match args.kind {
            Kind::Files => {
                if relative.contains(&args.query) {
                    all.push(json!(relative));
                }
            }
            Kind::Text | Kind::FileSize | Kind::Syntax => {
                let cap = if file_size || syntax {
                    let metadata = match entry.metadata() {
                        Ok(metadata) => metadata,
                        Err(_) => {
                            skipped += 1;
                            continue;
                        }
                    };
                    if metadata.len() > 2 * 1024 * 1024 {
                        skipped += 1;
                        continue;
                    }
                    if reserved_bytes + metadata.len() > 64 * 1024 * 1024 {
                        bounded = true;
                        break 'scan;
                    }
                    reserved_bytes += metadata.len();
                    metadata.len()
                } else {
                    2 * 1024 * 1024
                };
                let read = if file_size {
                    project.redacted_input_bytes(relative, context, cap)
                } else {
                    project.bytes_with_limit(relative, context, cap)
                };
                let file = match read {
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
                file_timing.bytes = file.len();
                if bytes > 64 * 1024 * 1024 {
                    bounded = true;
                    break;
                }
                if file.contains(&0) {
                    binary_skipped += 1;
                    if syntax {
                        skipped += 1;
                    }
                    continue;
                }
                let text = match std::str::from_utf8(&file) {
                    Ok(text) => text,
                    Err(_) if file_size => {
                        binary_skipped += 1;
                        continue;
                    }
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                };
                if let Some((kind, name)) = selected {
                    let digest = harness_domain::hash(&file);
                    let symbols = match super::rust_symbols::collect(text, &digest, context) {
                        Ok(symbols) => symbols,
                        Err((code, _)) if code == "CANCELLED" || code == "TIMEOUT" => {
                            cancelled = code == "CANCELLED";
                            timed_out = code == "TIMEOUT";
                            break 'scan;
                        }
                        Err(_) => {
                            skipped += 1;
                            parse_errors += 1;
                            continue;
                        }
                    };
                    stable_files += 1;
                    for symbol in symbols {
                        if let Err((code, _)) = context.check() {
                            cancelled = code == "CANCELLED";
                            timed_out = code == "TIMEOUT";
                            break 'scan;
                        }
                        if started.elapsed() >= Duration::from_secs(5) {
                            timed_out = true;
                            break 'scan;
                        }
                        if symbol.kind == kind && name.is_none_or(|name| symbol.name == name) {
                            let row = json!({"file":relative,"hash":digest,"name":symbol.name,
                                "container":symbol.container,"kind":symbol.kind,"start":symbol.start,
                                "end":symbol.end,"signature":symbol.signature,"symbolId":symbol.id});
                            metadata_bytes += row.to_string().len();
                            if metadata_bytes > 4 * 1024 * 1024 || all.len() >= 10000 {
                                bounded = true;
                                break 'scan;
                            }
                            all.push(row);
                        }
                    }
                    continue;
                }
                if file_size {
                    if started.elapsed() >= Duration::from_secs(5) {
                        timed_out = true;
                        break 'scan;
                    }
                    stable_files += 1;
                    let lines = file.iter().filter(|&&byte| byte == b'\n').count()
                        + usize::from(!file.is_empty() && file.last() != Some(&b'\n'));
                    if lines > min_lines {
                        let row = json!({"path":relative,"lines":lines,"bytes":file.len()});
                        metadata_bytes += row.to_string().len();
                        if metadata_bytes > 4 * 1024 * 1024 {
                            bounded = true;
                            break 'scan;
                        }
                        all.push(row);
                    }
                    continue;
                }
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
    if file_size && started.elapsed() >= Duration::from_secs(5) {
        timed_out = true;
    }
    if let Err((code, _)) = context.check() {
        cancelled |= code == "CANCELLED";
        timed_out |= code == "TIMEOUT";
    }
    let complete = !bounded && !timed_out && !cancelled && skipped == 0;
    if file_size {
        all.sort_by(|a, b| {
            b["lines"]
                .as_u64()
                .cmp(&a["lines"].as_u64())
                .then_with(|| b["bytes"].as_u64().cmp(&a["bytes"].as_u64()))
                .then_with(|| a["path"].as_str().cmp(&b["path"].as_str()))
        });
    }
    let mut compact: Vec<Value> = all.iter().take(limit).cloned().collect();
    if matches!(args.kind, Kind::Text) {
        for row in &mut compact {
            row["text"] = json!(prefix(row["text"].as_str().expect("match text"), 500).trim_end());
        }
    }
    let key = if file_size {
        "files"
    } else if matches!(args.kind, Kind::Files) {
        "paths"
    } else {
        "matches"
    };
    let mut folders: Vec<_> = folder_stats.into_inner().into_iter().map(|(path,(micros,files,bytes))|
        json!({"path":path,"processingMicros":micros,"files":files,"bytesRead":bytes})).collect();
    folders.sort_by_key(|row| std::cmp::Reverse(row["processingMicros"].as_u64().unwrap_or(0)));
    let processing: u64 = folders
        .iter()
        .map(|row| row["processingMicros"].as_u64().unwrap_or(0))
        .sum();
    let folder_count = folders.len();
    folders.truncate(8);
    let mut result = json!({"omitted":all.len().saturating_sub(compact.len()),"truncated":truncated || !complete,"_complete":complete,
        "cancelled":cancelled,"timedOut":timed_out,"complete":complete,"scanned":visited,"bytesRead":bytes,"skipped":skipped,"limited":bounded,
        "timings":{"executionMs":started.elapsed().as_millis(),"directoryProcessing":folders,
            "omittedFolders":folder_count.saturating_sub(8),
            "walkAndOtherMicros":(started.elapsed().as_micros().min(u64::MAX as u128) as u64).saturating_sub(processing),
            "coverage":"top 8 folder processing times, grouped to depth 2; traversal and other work reported separately"},
        "exclusions":[".git",".hg",".svn","node_modules","target","credential and secret paths"],
        "_detail":{},"diagnostics":[]});
    result[key] = json!(compact);
    result["_detail"][key] = json!(all);
    if let Some(patterns) = &args.globs {
        result["globs"] = json!(patterns);
    }
    if syntax {
        let coverage = "Rust declarations, inline modules, impl/trait members; no local items, macro expansion, references or AST patterns";
        result["kind"] = json!("syntax");
        result["language"] = json!("rust");
        result["syntaxOnly"] = json!(true);
        result["selector"] = json!(args.query);
        result["coverage"] = json!(coverage);
        result["parsedFiles"] = json!(stable_files);
        result["parseErrors"] = json!(parse_errors);
        result["limits"] = json!({"files":10000,"fileBytes":2*1024*1024,"totalBytes":64*1024*1024,"metadataBytes":4*1024*1024,"scanMs":5000});
        result["_detail"]["coverage"] = json!(coverage);
        result["_detail"]["syntaxOnly"] = json!(true);
        result["_detail"]["language"] = json!("rust");
    }
    if file_size {
        result["kind"] = json!("file_size");
        result["minLines"] = json!(min_lines);
        result["comparison"] = json!(">");
        result["visited"] = json!(visited);
        result["scanned"] = json!(stable_files);
        result["binarySkipped"] = json!(binary_skipped);
        result["counting"] = json!(
            "LF/CRLF totals including comments/tests; empty=0; trailing LF adds no extra line"
        );
        result["limits"] = json!({"files":10000,"fileBytes":2*1024*1024,"totalBytes":64*1024*1024,"metadataBytes":4*1024*1024,"scanMs":5000});
        result["binaryPolicy"] =
            json!("Skip NUL and invalid UTF-8 files without marking those skips incomplete");
        result["reservedBytes"] = json!(reserved_bytes);
    }
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
            external_scope: None,
            min_lines: None,
            globs: None,
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
            external_scope: None,
            min_lines: None,
            globs: None,
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
            external_scope: None,
            min_lines: None,
            globs: None,
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

    #[test]
    fn globs_filter_files_and_file_size_without_skipping() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/App.vue"), "needle\n").unwrap();
        fs::write(dir.path().join("src/main.ts"), "needle\n").unwrap();
        fs::write(dir.path().join("README.md"), "needle\n").unwrap();
        let p = Project::new(dir.path()).unwrap();
        let files = search(
            &p,
            &SearchArgs {
                external_scope: None,
                min_lines: None,
                globs: Some(vec!["*.vue".into()]),
                kind: Kind::Files,
                query: ".vue".into(),
                scope: None,
                limit: None,
                session: None,
            },
        )
        .unwrap();
        let paths = files["paths"].as_array().unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].as_str().unwrap().ends_with("App.vue"));
        assert_eq!(files["skipped"], 0);
        assert_eq!(files["complete"], true);
        assert_eq!(files["globs"], json!(["*.vue"]));
        let sizes = search(
            &p,
            &SearchArgs {
                external_scope: None,
                min_lines: Some(0),
                globs: Some(vec!["src/**".into()]),
                kind: Kind::FileSize,
                query: String::new(),
                scope: None,
                limit: None,
                session: None,
            },
        )
        .unwrap();
        assert_eq!(sizes["files"].as_array().unwrap().len(), 2);
        let rejected = search(
            &p,
            &SearchArgs {
                external_scope: None,
                min_lines: None,
                globs: Some(vec!["*.vue".into()]),
                kind: Kind::Text,
                query: "needle".into(),
                scope: None,
                limit: None,
                session: None,
            },
        )
        .unwrap_err();
        assert_eq!(rejected.0, "ARGUMENT");
    }
}

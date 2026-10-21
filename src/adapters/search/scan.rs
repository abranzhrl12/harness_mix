use super::super::{
    csharp_syntax, ignore as ignore_policy,
    project::{Failure, Project},
    rust_symbols, symbol_graph,
};
use super::graph::{self, GraphOutcome};
use super::plan::{SearchPlan, excluded};
use harness_domain::{
    contracts::{Kind, SearchArgs},
    execution::OperationContext,
    results::{prefix, utf16_len},
};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

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

pub(super) struct ScanOutcome<'a> {
    pub(super) file_size: bool,
    pub(super) syntax: bool,
    pub(super) context_lines: usize,
    pub(super) min_lines: usize,
    pub(super) limit: usize,
    pub(super) explicit_scope: bool,
    pub(super) scope: &'a str,
    pub(super) started: Instant,
    pub(super) folder_stats: RefCell<BTreeMap<String, (u64, usize, usize)>>,
    pub(super) all: Vec<Value>,
    pub(super) compact: Vec<Value>,
    pub(super) visited: usize,
    pub(super) bytes: usize,
    pub(super) reserved_bytes: u64,
    pub(super) skipped: usize,
    pub(super) skipped_files: Vec<Value>,
    pub(super) binary_skipped: usize,
    pub(super) parse_errors: usize,
    pub(super) stable_files: usize,
    pub(super) bounded: bool,
    pub(super) timed_out: bool,
    pub(super) cancelled: bool,
    pub(super) truncated: bool,
    pub(super) graph_outcome: Option<GraphOutcome>,
    pub(super) live_languages: BTreeSet<String>,
    pub(super) index_diagnostics: Vec<Value>,
    pub(super) complete: bool,
    pub(super) execution_ms: u128,
    pub(super) folders: Vec<Value>,
    pub(super) omitted_folders: usize,
    pub(super) walk_and_other_micros: u64,
    pub(super) custom_patterns: Option<usize>,
}

pub(super) fn execute<'a>(
    project: &Project,
    args: &'a SearchArgs,
    context: &OperationContext,
    plan: SearchPlan<'a>,
) -> Result<ScanOutcome<'a>, Failure> {
    let SearchPlan {
        file_size,
        syntax,
        context_lines,
        selected,
        globs,
        min_lines,
        limit,
        explicit_scope,
        scope,
        root,
    } = plan;
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
    // Defaults use override globs (higher precedence than ignore files) so a
    // project policy cannot re-enable dependency or build directories.
    let mut overrides = ignore::overrides::OverrideBuilder::new(&root);
    overrides
        .add("!node_modules/")
        .map_err(|_| ("INTERNAL", "Invalid default exclusion"))?;
    overrides
        .add("!target/")
        .map_err(|_| ("INTERNAL", "Invalid default exclusion"))?;
    builder.overrides(
        overrides
            .build()
            .map_err(|_| ("INTERNAL", "Invalid default exclusions"))?,
    );
    let custom_ignore = !explicit_scope;
    if custom_ignore {
        builder.add_custom_ignore_filename(ignore_policy::FILE);
    }
    let mut all = Vec::new();
    let mut visited = 0;
    let mut bytes = 0;
    let mut reserved_bytes = 0_u64;
    let mut skipped = 0;
    let mut skipped_files = Vec::new();
    let mut binary_skipped = 0;
    let mut parse_errors = 0;
    let mut stable_files = 0;
    let mut metadata_bytes = 0;
    let mut bounded = false;
    let mut timed_out = false;
    let mut cancelled = false;
    let mut truncated = false;
    // Symbol discovery prefers the graph index when it covers the scope: it is
    // bounded SQL instead of a full tree walk. Every candidate file is then
    // certified against the working tree and stale files are reparsed live, so
    // a published row is always true for the current source.
    let mut graph_outcome: Option<graph::GraphOutcome> = None;
    let mut live_languages = std::collections::BTreeSet::new();
    let mut index_diagnostics: Vec<Value> = Vec::new();
    if syntax {
        let (canonical, name) = selected.expect("syntax selector is present");
        match symbol_graph::discover(project, canonical, name, scope, limit, context) {
            Ok(Some(candidates)) => {
                let verdicts = symbol_graph::certify(project, &candidates, context);
                graph_outcome = Some(graph::graph_rows(
                    project,
                    &candidates,
                    &verdicts,
                    canonical,
                    name,
                    context,
                ));
                all = graph_outcome
                    .as_ref()
                    .map(|outcome| outcome.rows.clone())
                    .unwrap_or_default();
            }
            Ok(None) => {}
            Err((code, message)) => {
                index_diagnostics.push(json!({"code":code,"message":message}));
            }
        }
    }
    'scan: for entry in builder.build() {
        if graph_outcome.is_some() {
            // The graph already answered this query; the live walk is skipped.
            break 'scan;
        }
        let pending_path = entry
            .as_ref()
            .ok()
            .and_then(|entry| entry.path().strip_prefix(&project.root).ok())
            .map(|path| path.to_string_lossy().replace('\\', "/"));
        if let Err((code, _)) = context.check() {
            cancelled = code == "CANCELLED";
            timed_out = code == "TIMEOUT";
            skipped_files.push(json!({"path":pending_path,"reason":if cancelled { "cancelled" } else { "time_budget" }}));
            break;
        }
        if started.elapsed() >= Duration::from_secs(5) {
            timed_out = true;
            skipped_files.push(json!({"path":pending_path,"reason":"time_budget"}));
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                skipped += 1;
                skipped_files.push(json!({"path":pending_path,"reason":"walk_error"}));
                continue;
            }
        };
        if entry.error().is_some() {
            skipped += 1;
            skipped_files.push(json!({"path":pending_path,"reason":"walk_error"}));
        }
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        visited += 1;
        if visited > 10000 {
            bounded = true;
            skipped_files.push(json!({"path":pending_path,"reason":"result_budget"}));
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
                skipped_files.push(json!({"path":pending_path,"reason":"invalid_path"}));
                continue;
            }
        };
        if project.target(relative).is_err() {
            skipped += 1;
            skipped_files.push(json!({"path":pending_path,"reason":"path_rejected"}));
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
            && !entry.path().extension().is_some_and(|ext| {
                ext.eq_ignore_ascii_case("rs") || ext.eq_ignore_ascii_case("cshtml")
            })
        {
            continue;
        }
        match args.kind {
            Kind::Files => {
                if args.query.is_empty() || relative.contains(&args.query) {
                    all.push(json!(relative));
                }
            }
            Kind::Text | Kind::FileSize | Kind::Syntax => {
                let cap = if file_size || syntax {
                    let metadata = match entry.metadata() {
                        Ok(metadata) => metadata,
                        Err(_) => {
                            skipped += 1;
                            skipped_files
                                .push(json!({"path":pending_path,"reason":"metadata_error"}));
                            continue;
                        }
                    };
                    if metadata.len() > 2 * 1024 * 1024 {
                        skipped += 1;
                        skipped_files.push(json!({"path":pending_path,"reason":"file_size_limit"}));
                        continue;
                    }
                    if reserved_bytes + metadata.len() > 64 * 1024 * 1024 {
                        bounded = true;
                        skipped_files.push(json!({"path":pending_path,"reason":"result_budget"}));
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
                        skipped_files.push(json!({"path":pending_path,"reason":if cancelled { "cancelled" } else { "time_budget" }}));
                        break 'scan;
                    }
                    Err(_) => {
                        skipped += 1;
                        skipped_files.push(json!({"path":pending_path,"reason":"read_error"}));
                        continue;
                    }
                };
                bytes += file.len();
                file_timing.bytes = file.len();
                if bytes > 64 * 1024 * 1024 {
                    bounded = true;
                    skipped_files.push(json!({"path":pending_path,"reason":"result_budget"}));
                    break;
                }
                if file.contains(&0) {
                    binary_skipped += 1;
                    if syntax {
                        skipped += 1;
                        skipped_files.push(json!({"path":pending_path,"reason":"binary"}));
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
                        skipped_files.push(json!({"path":pending_path,"reason":"invalid_utf8"}));
                        continue;
                    }
                };
                if let Some((kind, name)) = selected {
                    let digest = harness_domain::hash(&file);
                    let collected = if entry
                        .path()
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("cshtml"))
                    {
                        csharp_syntax::collect_cshtml(text, &digest, context)
                    } else {
                        rust_symbols::collect(text, &digest, context)
                    };
                    let symbols = match collected {
                        Ok(symbols) => symbols,
                        Err((code, _)) if code == "CANCELLED" || code == "TIMEOUT" => {
                            cancelled = code == "CANCELLED";
                            timed_out = code == "TIMEOUT";
                            skipped_files.push(json!({"path":pending_path,"reason":if cancelled { "cancelled" } else { "time_budget" }}));
                            break 'scan;
                        }
                        Err(_) => {
                            skipped += 1;
                            skipped_files.push(json!({"path":pending_path,"reason":"parse_error"}));
                            parse_errors += 1;
                            continue;
                        }
                    };
                    stable_files += 1;
                    live_languages
                        .insert(graph::language_of(relative).unwrap_or_else(|| "rust".into()));
                    for symbol in symbols {
                        if let Err((code, _)) = context.check() {
                            cancelled = code == "CANCELLED";
                            timed_out = code == "TIMEOUT";
                            skipped_files.push(json!({"path":pending_path,"reason":if cancelled { "cancelled" } else { "time_budget" }}));
                            break 'scan;
                        }
                        if started.elapsed() >= Duration::from_secs(5) {
                            timed_out = true;
                            skipped_files.push(json!({"path":pending_path,"reason":"time_budget"}));
                            break 'scan;
                        }
                        if symbol_graph::canonical_kind(symbol.kind) == Some(kind)
                            && name.is_none_or(|name| symbol.name == name)
                        {
                            let row = json!({"file":relative,"hash":digest,"name":symbol.name,
                                "container":symbol.container,"kind":symbol.kind,"start":symbol.start,
                                "end":symbol.end,"signature":symbol.signature,"symbolId":symbol.id});
                            metadata_bytes += row.to_string().len();
                            if metadata_bytes > 4 * 1024 * 1024 || all.len() >= 10000 {
                                bounded = true;
                                skipped_files
                                    .push(json!({"path":pending_path,"reason":"result_budget"}));
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
                        skipped_files.push(json!({"path":pending_path,"reason":"time_budget"}));
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
                            skipped_files
                                .push(json!({"path":pending_path,"reason":"result_budget"}));
                            break 'scan;
                        }
                        all.push(row);
                    }
                    continue;
                }
                // Offsets refer to the bytes already read, so the hash and context describe
                // one source snapshot and preserve CRLF and trailing whitespace exactly.
                let offsets: Vec<usize> = if args.context_lines.is_some() {
                    std::iter::once(0)
                        .chain(text.match_indices('\n').map(|(offset, _)| offset + 1))
                        .filter(|offset| *offset < text.len())
                        .collect()
                } else {
                    Vec::new()
                };
                let digest = args.context_lines.map(|_| harness_domain::hash(&file));
                let source = text;
                for (line, text) in source.split_inclusive('\n').enumerate() {
                    if let Err((code, _)) = context.check() {
                        cancelled = code == "CANCELLED";
                        timed_out = code == "TIMEOUT";
                        skipped_files.push(json!({"path":pending_path,"reason":if cancelled { "cancelled" } else { "time_budget" }}));
                        break 'scan;
                    }
                    if started.elapsed() >= Duration::from_secs(5) {
                        timed_out = true;
                        skipped_files.push(json!({"path":pending_path,"reason":"time_budget"}));
                        break 'scan;
                    }
                    if text.contains(&args.query) {
                        truncated |= utf16_len(text) > 500;
                        let mut row = json!({"file":relative,"line":line + 1,"text":text});
                        if let Some(digest) = &digest {
                            let start = line.saturating_sub(context_lines);
                            let end = (line + context_lines + 1).min(offsets.len());
                            let end_offset = offsets.get(end).copied().unwrap_or(source.len());
                            row["hash"] = json!(digest);
                            row["context"] = json!({"start":start + 1,"end":end,
                                "text":&source[offsets[start]..end_offset]});
                        }
                        metadata_bytes += row.to_string().len();
                        if metadata_bytes > 4 * 1024 * 1024 {
                            bounded = true;
                            skipped_files
                                .push(json!({"path":pending_path,"reason":"result_budget"}));
                            break 'scan;
                        }
                        all.push(row);
                    }
                    // Bound stored results and a single file containing millions of lines.
                    if all.len() >= 10000 {
                        bounded = true;
                        skipped_files.push(json!({"path":pending_path,"reason":"result_budget"}));
                        break;
                    }
                }
            }
        }
        if all.len() >= 10000 {
            bounded = true;
            skipped_files.push(json!({"path":pending_path,"reason":"result_budget"}));
            break;
        }
    }
    Ok(ScanOutcome {
        file_size,
        syntax,
        context_lines,
        min_lines,
        limit,
        explicit_scope,
        scope,
        started,
        folder_stats,
        all,
        compact: Vec::new(),
        visited,
        bytes,
        reserved_bytes,
        skipped,
        skipped_files,
        binary_skipped,
        parse_errors,
        stable_files,
        bounded,
        timed_out,
        cancelled,
        truncated,
        graph_outcome,
        live_languages,
        index_diagnostics,
        complete: false,
        execution_ms: 0,
        folders: Vec::new(),
        omitted_folders: 0,
        walk_and_other_micros: 0,
        custom_patterns: None,
    })
}

pub(super) fn finalize<'a>(
    mut outcome: ScanOutcome<'a>,
    args: &SearchArgs,
    project: &Project,
    context: &OperationContext,
) -> ScanOutcome<'a> {
    if outcome.file_size && outcome.started.elapsed() >= Duration::from_secs(5) {
        outcome.timed_out = true;
    }
    if let Err((code, _)) = context.check() {
        outcome.cancelled |= code == "CANCELLED";
        outcome.timed_out |= code == "TIMEOUT";
    }
    outcome.complete =
        !outcome.bounded && !outcome.timed_out && !outcome.cancelled && outcome.skipped == 0;
    if outcome.file_size {
        outcome.all.sort_by(|a, b| {
            b["lines"]
                .as_u64()
                .cmp(&a["lines"].as_u64())
                .then_with(|| b["bytes"].as_u64().cmp(&a["bytes"].as_u64()))
                .then_with(|| a["path"].as_str().cmp(&b["path"].as_str()))
        });
    }
    outcome.compact = outcome.all.iter().take(outcome.limit).cloned().collect();
    if matches!(args.kind, Kind::Text) {
        for row in &mut outcome.compact {
            row["text"] = json!(prefix(row["text"].as_str().expect("match text"), 500).trim_end());
        }
    }
    let mut folders: Vec<_> = std::mem::take(&mut outcome.folder_stats)
        .into_inner()
        .into_iter()
        .map(|(path, (micros, files, bytes))| {
            json!({"path":path,"processingMicros":micros,"files":files,"bytesRead":bytes})
        })
        .collect();
    folders.sort_by_key(|row| std::cmp::Reverse(row["processingMicros"].as_u64().unwrap_or(0)));
    let processing: u64 = folders
        .iter()
        .map(|row| row["processingMicros"].as_u64().unwrap_or(0))
        .sum();
    let folder_count = folders.len();
    folders.truncate(8);
    outcome.omitted_folders = folder_count.saturating_sub(8);
    outcome.folders = folders;
    outcome.execution_ms = outcome.started.elapsed().as_millis();
    outcome.walk_and_other_micros = (outcome.started.elapsed().as_micros().min(u64::MAX as u128)
        as u64)
        .saturating_sub(processing);
    outcome.custom_patterns = if outcome.explicit_scope {
        None
    } else {
        ignore_policy::pattern_count(project, outcome.scope)
    };
    outcome
}

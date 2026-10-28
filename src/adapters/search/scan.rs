use super::super::{
    csharp_syntax, ignore as ignore_policy,
    project::{Failure, Project},
    rust_symbols, symbol_graph,
};
use super::graph::{self, GraphOutcome};
mod matches;
mod results;
mod walk;
use super::plan::SearchPlan;
use harness_domain::{
    contracts::{Kind, SearchArgs},
    execution::OperationContext,
    results::prefix,
};
use matches::Evaluation;
use results::MatchResults;
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

// Leave room for presentation within the operation's default 30-second deadline.
pub(super) const SCAN_TIMEOUT: Duration = Duration::from_secs(15);

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
        exclude_minified,
        syntax,
        context_lines,
        selected,
        globs,
        matcher,
        term_retrieval,
        structural,
        min_lines,
        limit,
        explicit_scope,
        scope,
        root,
    } = plan;
    let started = Instant::now();
    let mut text_searcher = grep_searcher::SearcherBuilder::new()
        .line_number(true)
        .bom_sniffing(false)
        .build();
    let extractor = if args.only_matching.unwrap_or(false) && matches!(args.kind, Kind::Text) {
        let pattern = if args.regex.unwrap_or(false) {
            args.query.clone()
        } else {
            regex::escape(&args.query)
        };
        regex::RegexBuilder::new(&pattern)
            .case_insensitive(!args.case_sensitive.unwrap_or(true))
            .build()
            .ok()
    } else {
        None
    };
    let folder_stats = RefCell::new(BTreeMap::new());
    let builder = walk::build(&root, exclude_minified, explicit_scope)?;
    let mut match_results = MatchResults::default();
    let mut visited = 0;
    let mut bytes = 0;
    let mut reserved_bytes = 0_u64;
    let mut skipped = 0;
    let mut skipped_files = Vec::new();
    let mut binary_skipped = 0;
    let mut parse_errors = 0;
    let mut stable_files = 0;
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
                match_results.rows = graph_outcome
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
        if started.elapsed() >= SCAN_TIMEOUT {
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
            skipped_files.push(json!({"path":pending_path,"reason":"file_count_budget"}));
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
        if structural
            .as_ref()
            .is_some_and(|search| !search.accepts(entry.path()))
        {
            continue;
        }
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
                    match_results.rows.push(json!(relative));
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
                        skipped_files
                            .push(json!({"path":pending_path,"reason":"total_bytes_budget"}));
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
                    skipped_files.push(json!({"path":pending_path,"reason":"total_bytes_budget"}));
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
                let decoded = match harness_domain::encoding::decode_source(&file) {
                    Ok(decoded) => decoded,
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
                let text = decoded.as_ref();
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
                    let mut state = matches::MatchState {
                        relative,
                        digest: &digest,
                        pending_path: &pending_path,
                        context,
                        started,
                        results: &mut match_results,
                        skipped_files: &mut skipped_files,
                        bounded: &mut bounded,
                        timed_out: &mut timed_out,
                        cancelled: &mut cancelled,
                        truncated: &mut truncated,
                        stable_files: &mut stable_files,
                    };
                    if matches::symbol_matches(symbols, kind, name, &mut state) == Evaluation::Stop
                    {
                        break 'scan;
                    }
                    continue;
                }
                if file_size {
                    let mut state = matches::MatchState {
                        relative,
                        digest: "",
                        pending_path: &pending_path,
                        context,
                        started,
                        results: &mut match_results,
                        skipped_files: &mut skipped_files,
                        bounded: &mut bounded,
                        timed_out: &mut timed_out,
                        cancelled: &mut cancelled,
                        truncated: &mut truncated,
                        stable_files: &mut stable_files,
                    };
                    if matches::file_size_matches(&file, min_lines, &mut state) == Evaluation::Stop
                    {
                        break 'scan;
                    }
                    continue;
                }
                let mut state = matches::MatchState {
                    relative,
                    digest: "",
                    pending_path: &pending_path,
                    context,
                    started,
                    results: &mut match_results,
                    skipped_files: &mut skipped_files,
                    bounded: &mut bounded,
                    timed_out: &mut timed_out,
                    cancelled: &mut cancelled,
                    truncated: &mut truncated,
                    stable_files: &mut stable_files,
                };
                let evaluation = if let Some(search) = &structural {
                    match matches::structural_matches(
                        text,
                        &file,
                        search,
                        context_lines,
                        &mut state,
                    ) {
                        Ok(evaluation) => evaluation,
                        Err(("PARSE", _)) => {
                            skipped += 1;
                            parse_errors += 1;
                            skipped_files.push(json!({"path":pending_path,"reason":"parse_error"}));
                            continue;
                        }
                        Err(error) => return Err(error),
                    }
                } else {
                    matches::text_matches(
                        text,
                        &file,
                        args.context_lines.is_some() || term_retrieval,
                        context_lines,
                        args.only_matching.unwrap_or(false),
                        extractor.as_ref(),
                        matcher.as_ref().expect("text matcher is present"),
                        &mut text_searcher,
                        &mut state,
                    )?
                };
                if evaluation == Evaluation::Stop {
                    break 'scan;
                }
            }
        }
        if match_results.rows.len() >= 10000 {
            bounded = true;
            skipped_files.push(json!({"path":pending_path,"reason":"match_count_budget"}));
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
        all: match_results.rows,
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
    if outcome.file_size && outcome.started.elapsed() >= SCAN_TIMEOUT {
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

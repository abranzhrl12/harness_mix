use super::super::{project::Project, symbol_graph};
use harness_domain::execution::OperationContext;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Rows published by the graph discovery path.
pub(super) struct GraphOutcome {
    pub(super) rows: Vec<Value>,
    pub(super) freshness: Value,
    pub(super) languages: Vec<String>,
    pub(super) certified_files: usize,
    pub(super) repaired_files: usize,
    pub(super) missing_files: usize,
}

/// Container of a qualified symbol name, without the symbol itself.
fn container_of(qualified: &str, name: &str) -> String {
    if qualified.is_empty() || name.is_empty() || !qualified.ends_with(name) {
        return String::new();
    }
    let prefix = &qualified[..qualified.len() - name.len()];
    prefix
        .trim_end_matches(':')
        .trim_end_matches('.')
        .to_owned()
}

/// Build published rows for graph candidates, reparsing stale files live.
pub(super) fn graph_rows(
    project: &Project,
    candidates: &[symbol_graph::Candidate],
    verdicts: &BTreeMap<String, symbol_graph::Verdict>,
    canonical: &str,
    name: Option<&str>,
    context: &OperationContext,
) -> GraphOutcome {
    let mut rows = Vec::new();
    let mut languages: Vec<String> = Vec::new();
    let mut repaired_files: Vec<String> = Vec::new();
    let mut missing_files: Vec<String> = Vec::new();
    let mut certified_files: Vec<String> = Vec::new();
    let mut repaired_cache: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for candidate in candidates {
        let verdict = verdicts
            .get(&candidate.file_path)
            .copied()
            .unwrap_or(symbol_graph::Verdict::Unknown);
        match verdict {
            symbol_graph::Verdict::Fresh => {
                if !certified_files.contains(&candidate.file_path) {
                    certified_files.push(candidate.file_path.clone());
                }
                let language = if candidate.language.is_empty() {
                    language_of(&candidate.file_path)
                } else {
                    Some(candidate.language.clone())
                };
                if let Some(language) = language
                    && !languages.contains(&language)
                {
                    languages.push(language);
                }
                rows.push(json!({
                    "file": candidate.file_path,
                    "hash": candidate.indexed_hash,
                    "name": candidate.name,
                    "container": container_of(&candidate.qualified_name, &candidate.name),
                    "kind": candidate.kind,
                    "start": candidate.start_line,
                    "end": candidate.end_line,
                    "signature": candidate.signature,
                    "doc": candidate.docstring,
                    "symbolId": candidate.id,
                    "freshness": "fresh",
                }));
            }
            symbol_graph::Verdict::Missing => {
                if !missing_files.contains(&candidate.file_path) {
                    missing_files.push(candidate.file_path.clone());
                }
            }
            symbol_graph::Verdict::Stale | symbol_graph::Verdict::Unknown => {
                if !repaired_files.contains(&candidate.file_path) {
                    repaired_files.push(candidate.file_path.clone());
                }
                let live = repaired_cache
                    .entry(candidate.file_path.clone())
                    .or_insert_with(|| {
                        live_symbols(project, &candidate.file_path, canonical, name, context)
                    });
                rows.extend(live.iter().cloned());
            }
        }
    }
    languages.sort();
    GraphOutcome {
        freshness: json!({
            "certifiedFiles": certified_files.len(),
            "repairedFiles": repaired_files.len(),
            "missingFiles": missing_files.len(),
            "verdicts": symbol_graph::freshness_summary(verdicts),
        }),
        rows,
        languages,
        certified_files: certified_files.len(),
        repaired_files: repaired_files.len(),
        missing_files: missing_files.len(),
    }
}

/// Reparse one file live and keep the symbols that match the selector.
fn live_symbols(
    project: &Project,
    relative: &str,
    canonical: &str,
    name: Option<&str>,
    context: &OperationContext,
) -> Vec<Value> {
    let Ok((_, digest, symbols, _, _)) =
        super::super::symbols::snapshot(project, relative, context, false)
    else {
        return Vec::new();
    };
    symbols
        .into_iter()
        .filter(|symbol| {
            symbol_graph::canonical_kind(symbol.kind) == Some(canonical)
                && name.is_none_or(|name| symbol.name == name)
        })
        .map(|symbol| {
            json!({
                "file": relative,
                "hash": digest,
                "name": symbol.name,
                "container": symbol.container,
                "kind": symbol.kind,
                "start": symbol.start,
                "end": symbol.end,
                "signature": symbol.signature,
                "doc": symbol.doc,
                "symbolId": symbol.id,
                "freshness": "repaired",
            })
        })
        .collect()
}

/// Language of a file, used to report which languages a query covered.
pub(super) fn language_of(relative: &str) -> Option<String> {
    let extension = relative.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(
        match extension.as_str() {
            "rs" => "rust",
            "cs" => "csharp",
            "ts" | "mts" | "cts" => "typescript",
            "tsx" => "tsx",
            "js" | "mjs" | "cjs" => "javascript",
            "jsx" => "jsx",
            "cshtml" | "razor" => "razor",
            "vue" => "vue",
            "html" | "htm" => "html",
            _ => return None,
        }
        .to_owned(),
    )
}

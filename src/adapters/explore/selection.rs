use super::model::Node;
use crate::adapters::project::Project;
use harness_domain::{Failure, contracts::ExploreArgs, execution::OperationContext};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(super) struct Selection {
    pub(super) symbol: String,
    pub(super) candidates: Vec<Node>,
    pub(super) candidates_omitted: usize,
    pub(super) selection_limited: bool,
    pub(super) truncated: bool,
    pub(super) files: BTreeSet<String>,
    pub(super) targets: Vec<Node>,
}

pub(super) enum SelectionResult {
    NotResolved,
    Selected(Selection),
}

pub(super) fn select<F>(
    db: &Connection,
    project: &Project,
    args: &ExploreArgs,
    file_path: Option<&str>,
    context: &OperationContext,
    symbol: &str,
    max_files: usize,
    has_qualified: bool,
    failed: F,
) -> Result<SelectionResult, Failure>
where
    F: Fn(rusqlite::Error) -> Failure,
{
    let query = if has_qualified {
        "SELECT id,name,kind,file_path,start_line FROM nodes WHERE (name IN (SELECT value FROM json_each(?1)) COLLATE BINARY OR (?1='[]' AND ?3 IS NOT NULL) OR (?5 AND kind='file')) AND (?2 IS NULL OR file_path=?2 COLLATE BINARY) AND (?3 IS NULL OR id=?3 COLLATE BINARY) AND (?4 IS NULL OR qualified_name=?4||'::'||name COLLATE BINARY OR qualified_name=?4||'.'||name COLLATE BINARY) ORDER BY file_path,start_line,id LIMIT 101"
    } else {
        "SELECT id,name,kind,file_path,start_line FROM nodes WHERE (name IN (SELECT value FROM json_each(?1)) COLLATE BINARY OR (?1='[]' AND ?3 IS NOT NULL) OR (?5 AND kind='file')) AND (?2 IS NULL OR file_path=?2 COLLATE BINARY) AND (?3 IS NULL OR id=?3 COLLATE BINARY) AND ?4 IS NULL ORDER BY file_path,start_line,id LIMIT 101"
    };
    let mut search = db.prepare(query).map_err(&failed)?;
    let symbol_names =
        serde_json::to_string(&super::query_symbols(args)).map_err(|_| super::CONTRACT)?;
    let read = |statement: &mut rusqlite::Statement<'_>| -> rusqlite::Result<Vec<Node>> {
        let params = rusqlite::params![
            symbol_names.as_str(),
            file_path,
            args.symbol_id,
            args.container,
            super::razor_file_selector(args)
        ];
        statement
            .query_map(params, |r| super::node(r, 0))?
            .collect()
    };
    let mut hits = read(&mut search).map_err(&failed)?;
    if hits.is_empty() && file_path.is_some() {
        // Selector drift (case differences): retry the file selector
        // case-insensitively before reporting the target unresolved. Native
        // separators arrive already normalized from the caller.
        let file_condition = if has_qualified {
            "AND (?3 IS NULL OR id=?3 COLLATE BINARY) AND (?4 IS NULL OR qualified_name=?4||'::'||name COLLATE BINARY OR qualified_name=?4||'.'||name COLLATE BINARY)"
        } else {
            "AND (?3 IS NULL OR id=?3 COLLATE BINARY) AND ?4 IS NULL"
        };
        let fallback = format!(
            "SELECT id,name,kind,file_path,start_line FROM nodes WHERE (name IN (SELECT value FROM json_each(?1)) COLLATE BINARY OR (?1='[]' AND ?3 IS NOT NULL) OR (?5 AND kind='file')) AND lower(file_path)=lower(?2) {file_condition} ORDER BY file_path,start_line,id LIMIT 101"
        );
        let mut retry = db.prepare(&fallback).map_err(&failed)?;
        hits = read(&mut retry).map_err(&failed)?;
    }
    if hits.is_empty()
        && (file_path.is_some() || args.container.is_some() || args.symbol_id.is_some())
    {
        return Ok(SelectionResult::NotResolved);
    }
    let mut truncated = hits.len() > 100;
    hits.truncate(100);
    let symbol = if symbol.is_empty() {
        hits.first().map(|hit| hit.name.clone()).unwrap_or_default()
    } else {
        symbol.to_owned()
    };
    let candidates: Vec<_> = hits.iter().take(20).cloned().collect();
    let candidates_omitted = hits.len().saturating_sub(candidates.len());
    let mut selection_limited = truncated;
    let mut files = BTreeSet::new();
    let mut targets = Vec::new();
    for mut hit in hits {
        context.check()?;
        super::validate_node(project, &mut hit)?;
        if !files.contains(&hit.file_path) && files.len() >= max_files || targets.len() >= 16 {
            truncated = true;
            selection_limited = true;
            continue;
        }
        files.insert(hit.file_path.clone());
        targets.push(hit);
    }
    Ok(SelectionResult::Selected(Selection {
        symbol,
        candidates,
        candidates_omitted,
        selection_limited,
        truncated,
        files,
        targets,
    }))
}

pub(super) fn not_resolved_value() -> Value {
    json!({"ok":false,"candidates":[],"diagnostics":[{"code":"TARGET_NOT_RESOLVED","message":"No indexed target matches all selectors; inspect outline or request candidates without the conflicting selector"}],"_complete":false})
}

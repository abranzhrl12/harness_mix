//! Narrow framework graph links whose targets are explicit indexed workspace files.
use crate::adapters::{index_inventory, project::Project};
use harness_domain::{Failure, execution::OperationContext, hash};
use rusqlite::{Connection, OptionalExtension};
use std::collections::BTreeMap;

const FRAMEWORK_ID_PREFIX: &str = "native:framework:";
const MAX_SNAPSHOT_FILES: usize = 1024;
const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;

pub(in crate::adapters::index_store) fn is_native_framework_id(id: &str) -> bool {
    id.starts_with(FRAMEWORK_ID_PREFIX)
}

pub(in crate::adapters::index_store) fn is_template_reference(kind: &str) -> bool {
    kind == "framework_template"
}

pub(in crate::adapters::index_store) struct TemplateTarget {
    pub(in crate::adapters::index_store) node_id: String,
    pub(in crate::adapters::index_store) target_path: String,
    pub(in crate::adapters::index_store) source_hash: String,
    pub(in crate::adapters::index_store) target_hash: String,
}

#[derive(Default)]
pub(in crate::adapters::index_store) struct Snapshot {
    files: BTreeMap<String, String>,
    reserved: usize,
}

impl Snapshot {
    pub(in crate::adapters::index_store) fn remember(
        &mut self,
        path: &str,
        digest: &str,
    ) -> Result<(), Failure> {
        if let Some(existing) = self.files.get(path) {
            if existing != digest {
                return Err((
                    "STALE",
                    "Framework template source changed during resolution",
                ));
            }
            return Ok(());
        }
        let reserved = path.len() + digest.len();
        if self.files.len() >= MAX_SNAPSHOT_FILES
            || self.reserved.saturating_add(reserved) > MAX_SNAPSHOT_BYTES
        {
            return Err(("SYNC_LIMIT", "Framework template snapshot budget exceeded"));
        }
        self.reserved += reserved;
        self.files.insert(path.to_owned(), digest.to_owned());
        Ok(())
    }

    pub(in crate::adapters::index_store) fn recheck(
        &self,
        project: &Project,
        context: &OperationContext,
    ) -> Result<(), Failure> {
        for (path, digest) in &self.files {
            context.check()?;
            let current = super::super::read_source(project, path, context)?;
            if current.as_ref().map(|bytes| hash(bytes)) != Some(digest.clone()) {
                return Err((
                    "STALE",
                    "Framework template source changed before publication; transaction rolled back",
                ));
            }
        }
        Ok(())
    }
}

/// Resolve only a normalized, already-indexed HTML target referenced by an
/// indexed JS/TS component. Disk bytes must still match both snapshots.
pub(in crate::adapters::index_store) fn indexed_template_target(
    db: &Connection,
    project: &Project,
    source_node: &str,
    source_path: &str,
    source_language: &str,
    target_path: &str,
    context: &OperationContext,
) -> Result<Option<TemplateTarget>, Failure> {
    context.check()?;
    if !matches!(source_language, "typescript" | "tsx" | "javascript" | "jsx")
        || !valid_workspace_path(source_path)
        || index_inventory::restricted(source_path)
        || !valid_template_path(target_path)
        || !is_native_framework_id(source_node)
    {
        return Ok(None);
    }
    let source_node_valid: bool = sql(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM nodes WHERE id=?1 AND file_path=?2 AND language=?3 AND kind='component')",
        rusqlite::params![source_node, source_path, source_language],
        |row| row.get(0),
    ))?;
    if !source_node_valid {
        return Ok(None);
    }
    let Some(source_hash) = file_hash(db, source_path, source_language)? else {
        return Ok(None);
    };
    let Some(target_hash) = file_hash(db, target_path, "html")? else {
        return Ok(None);
    };
    let source_bytes = match super::super::read_source(project, source_path, context) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Ok(None),
        Err(error @ ("CANCELLED" | "TIMEOUT", _)) => return Err(error),
        Err(_) => return Ok(None),
    };
    let target_bytes = match super::super::read_source(project, target_path, context) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Ok(None),
        Err(error @ ("CANCELLED" | "TIMEOUT", _)) => return Err(error),
        Err(_) => return Ok(None),
    };
    if hash(&source_bytes) != source_hash || hash(&target_bytes) != target_hash {
        return Ok(None);
    }
    let mut statement = sql(db.prepare(
        "SELECT id FROM nodes WHERE file_path=?1 AND kind='file' AND language='html' LIMIT 2",
    ))?;
    let mut rows = sql(statement.query_map([target_path], |row| row.get::<_, String>(0)))?;
    let Some(node_id) = rows
        .next()
        .transpose()
        .map_err(|_| ("INDEX", "Index unavailable"))?
    else {
        return Ok(None);
    };
    if rows
        .next()
        .transpose()
        .map_err(|_| ("INDEX", "Index unavailable"))?
        .is_some()
    {
        return Ok(None);
    }
    Ok(Some(TemplateTarget {
        node_id,
        target_path: target_path.to_owned(),
        source_hash,
        target_hash,
    }))
}

fn file_hash(db: &Connection, path: &str, language: &str) -> Result<Option<String>, Failure> {
    sql(db
        .query_row(
            "SELECT content_hash FROM files WHERE path=?1 AND language=?2",
            rusqlite::params![path, language],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional())
    .map(|hash| hash.flatten())
}

fn valid_template_path(path: &str) -> bool {
    valid_workspace_path(path)
        && std::path::Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("html") || extension.eq_ignore_ascii_case("htm")
            })
        && !index_inventory::restricted(path)
}

fn valid_workspace_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1000
        && !path.contains(['\\', ':'])
        && !path.chars().any(char::is_control)
        && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
}

fn sql<T>(value: rusqlite::Result<T>) -> Result<T, Failure> {
    value.map_err(|_| ("INDEX", "Index unavailable"))
}

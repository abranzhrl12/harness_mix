use super::repository::{
    ProviderEdge, framework, identities, identity_map, incoming, insert_ref, provider_edges,
    provider_node_ids,
};
use super::resolution::resolve_exact;
use super::{
    INDEX, LIMIT, MAX_ROWS, batches, candidates, imports, index_extract, razor_invalidation, sql,
    sync_diagnostic, tsconfig,
};
use crate::adapters::project::Project;
use harness_domain::{Failure, execution::OperationContext, hash};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    time::{SystemTime, UNIX_EPOCH},
};

#[path = "publication/model.rs"]
mod model;
#[path = "publication/plan.rs"]
mod plan;
#[path = "publication/retry.rs"]
mod retry;
#[path = "publication/storage.rs"]
mod storage;

#[allow(clippy::too_many_arguments)] // Publication inputs plus the transaction's configuration snapshot.
pub(super) fn publish(
    db: &Transaction<'_>,
    project: &Project,
    path: &str,
    language: &str,
    bytes: Option<&[u8]>,
    context: &OperationContext,
    configuration: (tsconfig::Snapshot, bool),
    force: bool,
    prepared: Option<super::parallel::Prepared>,
    retry_memo: &mut super::resolution::RetryMemo,
) -> Result<(Value, imports::Sources), Failure> {
    let (configuration, config_changed) = configuration;
    sync_diagnostic::stage("check_indexed_hash");
    let record: Option<(String, Option<String>)> = sql(db
        .query_row(
            "SELECT content_hash,errors FROM files WHERE path=?1",
            [path],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional())?;
    if plan::is_unchanged(record.as_ref(), bytes, config_changed, force, hash) {
        return Ok((
            model::PublicationReport::unchanged(path),
            imports::Sources::configured(configuration),
        ));
    }
    // Parse only after the authoritative transaction sees a changed digest or
    // an extraction retry/configuration change. Duplicate watcher events are cheap.
    sync_diagnostic::stage("extract_source");
    let mut prepared = prepared;
    let extracted_value = bytes
        .map(|bytes| {
            if let Some(result) = prepared.take().and_then(|value| value.consume(bytes)) {
                return result;
            }
            let text =
                std::str::from_utf8(bytes).map_err(|_| ("ENCODING", "Source is not UTF-8"))?;
            index_extract::extract(path, text, context)
        })
        .transpose()?;
    let extracted = extracted_value.as_ref();
    sync_diagnostic::stage("capture_existing_graph");
    let before_declarations = if retry_memo.enabled() {
        retry::declarations(db, path)?
    } else {
        None
    };
    let razor_changes = if language == "csharp" {
        Some(razor_invalidation::Changes::capture(db, path, extracted)?)
    } else {
        None
    };
    // Provider-owned framework nodes (routes, components) and the edges the
    // provider synthesized around them cannot be reproduced by the native
    // extractor. The publication keeps them and writes the native nodes of
    // this file next to them. Only provider data that the native publisher
    // cannot reproduce at all still blocks the transaction.
    let provider_ids: BTreeSet<String> = provider_node_ids(db, path)?;
    let synthesized:bool = sql(db.query_row("SELECT EXISTS(SELECT 1 FROM edges WHERE (source IN(SELECT id FROM nodes WHERE file_path=?1 AND (kind NOT IN('route','component') OR id LIKE 'native:framework:%')) OR target IN(SELECT id FROM nodes WHERE file_path=?1 AND (kind NOT IN('route','component') OR id LIKE 'native:framework:%'))) AND CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.synthesizedBy') IS NOT NULL ELSE 0 END)",[path],|r|r.get(0)))?;
    let third_file:bool = sql(db.query_row("SELECT EXISTS(SELECT 1 FROM edges WHERE CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.synthesizedBy') END IS NOT NULL AND CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.registeredAt') END >= ?1 AND CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.registeredAt') END < ?2)",params![format!("{path}:"),format!("{path};")],|r|r.get(0)))?;
    let synthesis_table:bool = sql(db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='synthesis_inputs' AND upper(sql) LIKE 'CREATE TABLE%')",[],|r|r.get(0)))?;
    let _prior_synthesis_input = if synthesis_table {
        sql(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM synthesis_inputs WHERE file_path=?1)",
            [path],
            |r| r.get::<_, bool>(0),
        ))?
    } else {
        false
    };
    // A file the provider listed as a synthesis input can still be published
    // natively: its provider nodes and edges are preserved and the recorded
    // input stays, so the provider can synthesize again. Only a provider
    // relation that points at the native nodes of this file blocks it.
    if synthesized || third_file {
        return Err((
            "SYNC_UNSUPPORTED_RELATION",
            "Affected framework/synthesized graph data requires the provider resolver; index unchanged",
        ));
    }
    // Provider nodes and their edges survive the publication.
    let provider_edges: Vec<ProviderEdge> = provider_edges(db, path)?;
    let prior = identities(db, path)?;
    let paired = identity_map(prior, extracted)?;
    let incoming = incoming(db, path)?;
    // Capture original refs BEFORE deleting nodes/edges; never turn receiver.foo
    // into foo. Legacy unpaired edges without stamps block the entire transaction.
    let mut resurrect = Vec::new();
    let mut retained = Vec::new();
    for edge in incoming {
        context.check()?;
        match plan::incoming_disposition(edge, &provider_ids, &paired)? {
            plan::IncomingDisposition::Retain(edge) => retained.push(edge),
            plan::IncomingDisposition::Recover { edge, name, kind } => {
                resurrect.push((edge, name, kind));
            }
        }
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| INDEX)?
        .as_millis() as i64;
    let generated: i64 = sql(db
        .query_row("SELECT generated FROM files WHERE path=?1", [path], |r| {
            r.get(0)
        })
        .optional())?
    .unwrap_or(0);
    // Explicit deletion also supports indexes produced with foreign_keys off.
    // Provider nodes and their pending references survive the publication.
    sync_diagnostic::stage("delete_previous_graph");
    sql(db.execute("DELETE FROM unresolved_refs WHERE from_node_id IN(SELECT id FROM nodes WHERE file_path=?1 AND (kind NOT IN('route','component') OR id LIKE 'native:framework:%'))",[path]))?;
    sql(db.execute("DELETE FROM edges WHERE source IN(SELECT id FROM nodes WHERE file_path=?1) OR target IN(SELECT id FROM nodes WHERE file_path=?1)",[path]))?;
    sql(db.execute(
        "DELETE FROM nodes WHERE file_path=?1 AND (kind NOT IN('route','component') OR id LIKE 'native:framework:%')",
        [path],
    ))?;
    let prepared_graph = extracted
        .map(|value| model::ExtractedGraph::prepare(value, context))
        .transpose()?;
    let extraction_performed = extracted.is_some();
    sync_diagnostic::stage("write_nodes_and_edges");
    let graph_write = storage::replace_file_graph(
        db,
        storage::FilePublication {
            graph: prepared_graph.as_ref(),
            path,
            language,
            bytes,
            generated,
            now,
            provider_nodes: provider_ids.len(),
        },
        context,
    )?;
    let node_count = graph_write.node_count;
    sync_diagnostic::stage("restore_incoming_edges");
    let mut refs = graph_write.refs;
    let retained_count = retained.len();
    let resurrected_count = resurrect.len();
    batches::insert(
        db,
        "INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES ",
        7,
        &retained,
        context,
        |edge| {
            batches::values(params![
                edge.source,
                edge.target,
                edge.kind,
                edge.line,
                edge.col,
                edge.metadata,
                edge.provenance
            ])
        },
    )?;
    for (edge, name, kind) in resurrect {
        context.check()?;
        refs.insert(insert_ref(
            db,
            &edge.source,
            &name,
            &kind,
            edge.line.unwrap_or(0),
            edge.col.unwrap_or(0),
            None,
            (&edge.path, &edge.language),
        )?);
        if refs.len() > MAX_ROWS {
            return Err(LIMIT);
        }
    }
    // Provider-owned edges are republished with their native endpoints remapped.
    // Edges that arrive from another file are already retained above.
    let mut provider_kept = 0usize;
    let mut provider_dropped = 0usize;
    for edge in &provider_edges {
        context.check()?;
        if edge.source_path != path {
            continue;
        }
        let source = if edge.source_path != path {
            edge.source.clone()
        } else if provider_ids.contains(&edge.source) {
            edge.source.clone()
        } else if let Some(next) = paired.get(&edge.source) {
            next.clone()
        } else {
            provider_dropped += 1;
            continue;
        };
        let target = if edge.target_path != path {
            edge.target.clone()
        } else if provider_ids.contains(&edge.target) {
            edge.target.clone()
        } else if let Some(next) = paired.get(&edge.target) {
            next.clone()
        } else {
            provider_dropped += 1;
            continue;
        };
        let duplicate: bool = sql(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM edges WHERE source=?1 AND target=?2 AND kind=?3)",
            params![source, target, edge.kind],
            |r| r.get(0),
        ))?;
        if duplicate {
            continue;
        }
        sql(db.execute("INSERT INTO edges(source,target,kind,line,col,metadata,provenance) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![source,target,edge.kind,edge.line,edge.col,edge.metadata,edge.provenance]))?;
        provider_kept += 1;
    }
    // A changed JS/TS source or configuration can make bindings elsewhere
    // ambiguous. Preserve global import invalidation for that subset instead
    // of limiting retries to the current file's incoming and outgoing edges.
    // Other languages with unchanged configuration do not trigger this pass.

    sync_diagnostic::stage("invalidate_references");
    let after_declarations = if retry_memo.enabled() {
        retry::declarations(db, path)?
    } else {
        None
    };
    let declaration_pool_changed = config_changed
        || before_declarations
            .as_ref()
            .zip(after_declarations.as_ref())
            .is_none_or(|(before, after)| before != after);
    if declaration_pool_changed {
        retry_memo.clear();
    }
    let (import_rechecks, framework_rechecks) = imports::invalidate_publication(
        db,
        config_changed || matches!(language, "typescript" | "tsx" | "javascript" | "jsx"),
        config_changed
            || matches!(
                language,
                "typescript" | "tsx" | "javascript" | "jsx" | "vue"
            ),
    )?;
    let import_recheck_count = import_rechecks.len();
    refs.extend(import_rechecks);
    let framework_recheck_count = framework_rechecks.len();
    refs.extend(framework_rechecks);
    if language == "rust" {
        refs.extend(imports::invalidate_rust(db)?);
    }
    let razor_recheck_count = if let Some(changes) = &razor_changes {
        let rechecks = changes.recheck(db, context)?;
        let count = rechecks.len();
        refs.extend(rechecks);
        count
    } else {
        0
    };
    if refs.len() > MAX_ROWS {
        return Err(LIMIT);
    }
    // Retry the pending refs whose exact name became available in this file.
    // The failed ones are retried by the engine that produced them (the razor
    // tail index, the import invalidation). The project-wide import clause
    // only applies to a JS/TS publication: a C# or Razor source cannot change
    // the resolution of a JavaScript import.
    let import_bindings =
        if config_changed || matches!(language, "typescript" | "tsx" | "javascript" | "jsx") {
            imports::retry_binding_catalog(db)?
        } else {
            None
        };
    for id in retry::pending(
        db,
        path,
        language,
        config_changed,
        &retry_memo.json(),
        import_bindings.as_deref(),
    )? {
        refs.insert(id);
        if refs.len() > MAX_ROWS {
            return Err(LIMIT);
        }
    }
    let mut candidate_snapshot = candidates::Snapshot::default();
    // Registration refs do not necessarily share an exported class's local
    // alias. Retry them on module/config changes, including previously missing
    // targets and negative export branches. No global same-name fallback.
    if config_changed || matches!(language, "typescript" | "tsx" | "javascript" | "jsx") {
        let mut provider_refs = sql(db.prepare(
            "SELECT id FROM unresolved_refs WHERE status='pending' AND reference_kind IN ('angular_provider_token','angular_provider_implementation') LIMIT 20001",
        ))?;
        for row in sql(provider_refs.query_map([], |row| row.get::<_, i64>(0)))? {
            refs.insert(sql(row)?);
            if refs.len() > MAX_ROWS {
                return Err(LIMIT);
            }
        }
    }
    let mut resolved = 0;
    let mut import_sources = imports::Sources::configured(configuration);
    let mut mvc_snapshot = super::super::index_razor_mvc::Snapshot::default();
    let mut framework_snapshot = framework::Snapshot::default();
    sync_diagnostic::stage("resolve_references");
    for id in &refs {
        context.check()?;
        resolved += usize::from(resolve_exact(
            db,
            *id,
            project,
            context,
            &mut import_sources,
            &mut candidate_snapshot,
            &mut mvc_snapshot,
            &mut framework_snapshot,
        )?);
    }
    if matches!(language, "typescript" | "tsx" | "javascript" | "jsx")
        && let Some(declarations) = &after_declarations
    {
        retry_memo.remember(retry::visible_names(declarations));
    }
    sync_diagnostic::stage("recheck_framework_sources");
    framework_snapshot.recheck(project, context)?;
    sync_diagnostic::stage("write_metadata");
    if import_sources.config.digest.is_some()
        || import_sources
            .config
            .semantic
            .as_ref()
            .is_some_and(|p| p.stamp().is_some())
    {
        sql(db.execute("INSERT INTO project_metadata(key,value,updated_at) VALUES(?1,?2,?3) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at", params![tsconfig::KEY, import_sources.config.stamp().to_string(), now]))?;
    } else {
        sql(db.execute("DELETE FROM project_metadata WHERE key=?1", [tsconfig::KEY]))?;
    }
    // Invalidate the global provider stamp. This single-file subset is not a
    // full language/framework resolver or a whole-workspace synchronization.
    sql(db.execute(
        "DELETE FROM project_metadata WHERE key IN('indexed_at_commit','indexed_dirty_paths')",
        [],
    ))?;
    sql(db.execute("INSERT INTO project_metadata(key,value,updated_at) VALUES('native_sync_state',?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",params![json!({"scope":"explicit_file","resolution":"exact_name_and_relative_named_import_subset","globalFreshnessCertified":false}).to_string(),now]))?;
    let mut report = model::PublicationReport {
        path: path.to_owned(),
        changed: true,
        extraction_performed,
        deleted: extracted.is_none(),
        nodes_written: node_count,
        incoming_edges_retained: retained_count,
        incoming_references_recovered: resurrected_count,
        provider_nodes_preserved: provider_ids.len(),
        provider_edges_retained: provider_kept,
        provider_edges_dropped: provider_dropped,
        import_references_rechecked: import_recheck_count,
        framework_references_rechecked: framework_recheck_count,
        razor_references_rechecked: razor_recheck_count,
        references_attempted: refs.len(),
        references_resolved: resolved,
        extraction_coverage: prepared_graph
            .as_ref()
            .and_then(|graph| graph.coverage)
            .cloned(),
    }
    .into_json();
    if std::env::var("HARNESS_INDEX_PROFILE").as_deref() == Ok("1") {
        report["retryMemoNames"] = json!(retry_memo.retained_names());
        report["retryMemoDataVersion"] = json!(retry_memo.data_version());
        report["declarationPoolChanged"] = json!(declaration_pool_changed);
        report["configurationChanged"] = json!(config_changed);
    }
    Ok((report, import_sources))
}

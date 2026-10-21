use super::repository::{
    ProviderEdge, framework, identities, identity_map, incoming, insert_ref, provider_edges,
    provider_node_ids,
};
use super::resolution::resolve_exact;
use super::{
    INDEX, LIMIT, MAX_ROWS, array, batches, candidates, imports, index_extract, opt_string,
    razor_invalidation, sql, string, tsconfig,
};
use crate::adapters::project::Project;
use harness_domain::{Failure, execution::OperationContext, hash};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    time::{SystemTime, UNIX_EPOCH},
};

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
) -> Result<(Value, imports::Sources), Failure> {
    let (configuration, config_changed) = configuration;
    let record: Option<(String, Option<String>)> = sql(db
        .query_row(
            "SELECT content_hash,errors FROM files WHERE path=?1",
            [path],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional())?;
    let clean = record.as_ref().is_some_and(|(_, errors)| {
        errors.as_ref().is_none_or(|e| {
            serde_json::from_str::<Value>(e).is_ok_and(|v| v.as_array().is_some_and(Vec::is_empty))
        })
    });
    if !config_changed
        && !force
        && ((bytes.is_none() && record.is_none())
            || (clean
                && bytes.is_some_and(|bytes| {
                    record
                        .as_ref()
                        .is_some_and(|(digest, _)| *digest == hash(bytes))
                })))
    {
        return Ok((
            json!({"mode":"sync_file","path":path,"changed":false,"extractionPerformed":false,"committed":false,"synchronized":false,"globalFreshnessCertified":false}),
            imports::Sources::configured(configuration),
        ));
    }
    // Parse only after the authoritative transaction sees a changed digest or
    // an extraction retry/configuration change. Duplicate watcher events are cheap.
    let extracted_value = bytes
        .map(|bytes| {
            let text =
                std::str::from_utf8(bytes).map_err(|_| ("ENCODING", "Source is not UTF-8"))?;
            index_extract::extract(path, text, context)
        })
        .transpose()?;
    let extracted = extracted_value.as_ref();
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
    // A name that other files resolve against is a name this file publishes.
    // A pending ref can only be unblocked by a name that appears now or that
    // becomes published now.
    let mut prior_published: BTreeSet<String> = BTreeSet::new();
    {
        let mut statement = sql(db.prepare(
            "SELECT name FROM nodes WHERE file_path=?1 AND ifnull(is_exported,0)=1 LIMIT 20001",
        ))?;
        let rows = sql(statement.query_map([path], |r| r.get::<_, String>(0)))?;
        for row in rows {
            prior_published.insert(sql(row)?);
        }
    }
    let incoming = incoming(db, path)?;
    // Capture original refs BEFORE deleting nodes/edges; never turn receiver.foo
    // into foo. Legacy unpaired edges without stamps block the entire transaction.
    let mut resurrect = Vec::new();
    let mut retained = Vec::new();
    for mut edge in incoming {
        context.check()?;
        let framework_template = edge
            .metadata
            .as_deref()
            .and_then(|metadata| serde_json::from_str::<Value>(metadata).ok())
            .and_then(|metadata| opt_string(&metadata, "refKind").map(str::to_owned))
            .is_some_and(|kind| framework::is_template_reference(&kind));
        if framework_template {
            // Revalidate even when the target's node identity is unchanged:
            // the edge metadata records both contributing file snapshots.
            let metadata: Value = edge
                .metadata
                .as_ref()
                .and_then(|s| serde_json::from_str(s).ok())
                .ok_or((
                    "SYNC_LEGACY_REFERENCE",
                    "Cannot recover framework template reference; index unchanged",
                ))?;
            let name = string(&metadata, "refName").map_err(|_| {
                (
                    "SYNC_LEGACY_REFERENCE",
                    "Framework template edge lacks original path; index unchanged",
                )
            })?;
            resurrect.push((edge, name, "framework_template".to_owned()));
            continue;
        }
        if provider_ids.contains(&edge.target) {
            // The provider node survives the publication; the edge keeps it.
            retained.push(edge);
            continue;
        }
        if let Some(target) = paired.get(&edge.target) {
            edge.target = target.clone();
            retained.push(edge);
        } else {
            let metadata: Value = edge
                .metadata
                .as_ref()
                .and_then(|s| serde_json::from_str(s).ok())
                .ok_or((
                    "SYNC_LEGACY_REFERENCE",
                    "Cannot recover original reference for an incoming edge; index unchanged",
                ))?;
            let name = string(&metadata, "refName").map_err(|_| {
                (
                    "SYNC_LEGACY_REFERENCE",
                    "Incoming edge lacks original reference metadata; index unchanged",
                )
            })?;
            let kind = opt_string(&metadata, "refKind")
                .unwrap_or(&edge.kind)
                .to_owned();
            resurrect.push((edge, name, kind));
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
    sql(db.execute("DELETE FROM unresolved_refs WHERE from_node_id IN(SELECT id FROM nodes WHERE file_path=?1 AND (kind NOT IN('route','component') OR id LIKE 'native:framework:%'))",[path]))?;
    sql(db.execute("DELETE FROM edges WHERE source IN(SELECT id FROM nodes WHERE file_path=?1) OR target IN(SELECT id FROM nodes WHERE file_path=?1)",[path]))?;
    sql(db.execute(
        "DELETE FROM nodes WHERE file_path=?1 AND (kind NOT IN('route','component') OR id LIKE 'native:framework:%')",
        [path],
    ))?;
    let mut refs = BTreeSet::new();
    let mut node_count = 0;
    if let Some(extracted) = extracted {
        let nodes = array(extracted, "nodes")?;
        if nodes.len() > MAX_ROWS {
            return Err(LIMIT);
        }
        batches::insert(
            db,
            "INSERT INTO nodes(id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,docstring,signature,visibility,is_exported,is_async,is_static,is_abstract,decorators,type_parameters,return_type,updated_at) VALUES ",
            21,
            nodes,
            context,
            |node| {
                batches::values(params![
                    string(node, "id")?,
                    string(node, "kind")?,
                    string(node, "name")?,
                    string(node, "qualifiedName")?,
                    path,
                    language,
                    node["startLine"].as_i64(),
                    node["endLine"].as_i64(),
                    node["startColumn"].as_i64(),
                    node["endColumn"].as_i64(),
                    opt_string(node, "docstring"),
                    opt_string(node, "signature"),
                    opt_string(node, "visibility"),
                    node["isExported"].as_bool().unwrap_or(false),
                    node["isAsync"].as_bool().unwrap_or(false),
                    node["isStatic"].as_bool().unwrap_or(false),
                    node["isAbstract"].as_bool().unwrap_or(false),
                    node.get("decorators").map(Value::to_string),
                    node.get("typeParameters").map(Value::to_string),
                    opt_string(node, "returnType"),
                    now
                ])
            },
        )?;
        node_count = nodes.len() + provider_ids.len();
        batches::insert(
            db,
            "INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES ",
            7,
            array(extracted, "edges")?,
            context,
            |edge| {
                batches::values(params![
                    string(edge, "source")?,
                    string(edge, "target")?,
                    string(edge, "kind")?,
                    edge["line"].as_i64(),
                    edge["column"].as_i64(),
                    edge.get("metadata").map(Value::to_string),
                    opt_string(edge, "provenance")
                ])
            },
        )?;
        refs.extend(batches::insert_ids(
            db,
            "INSERT INTO unresolved_refs(from_node_id,reference_name,reference_kind,line,col,candidates,file_path,language) VALUES ",
            8,
            array(extracted, "unresolvedReferences")?,
            context,
            |reference| batches::values(params![
                string(reference, "fromNodeId")?,
                string(reference, "referenceName")?,
                string(reference, "referenceKind")?,
                reference["line"].as_i64().ok_or(INDEX)?,
                reference["column"].as_i64().ok_or(INDEX)?,
                reference.get("candidates").map(Value::to_string),
                path, language,
            ]),
        )?);
        if refs.len() > MAX_ROWS {
            return Err(LIMIT);
        }
        let bytes = bytes.ok_or(INDEX)?;
        // modified_at=0 disables CodeGraph's size/mtime fast-path on the next
        // provider sync; don't invent a file timestamp from extraction time.
        sql(db.execute("INSERT INTO files(path,content_hash,language,size,modified_at,indexed_at,node_count,errors,generated) VALUES(?1,?2,?3,?4,0,?5,?6,'[]',?7) ON CONFLICT(path) DO UPDATE SET content_hash=excluded.content_hash,language=excluded.language,size=excluded.size,modified_at=0,indexed_at=excluded.indexed_at,node_count=excluded.node_count,errors=excluded.errors,generated=excluded.generated",params![path,hash(bytes),language,bytes.len() as i64,now,node_count as i64,generated]))?;
    } else {
        sql(db.execute("DELETE FROM files WHERE path=?1", [path]))?;
    }
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

    let import_rechecks =
        if config_changed || matches!(language, "typescript" | "tsx" | "javascript" | "jsx") {
            imports::invalidate(db)?
        } else {
            Vec::new()
        };
    let import_recheck_count = import_rechecks.len();
    refs.extend(import_rechecks);
    let framework_rechecks = if config_changed
        || matches!(
            language,
            "typescript" | "tsx" | "javascript" | "jsx" | "vue"
        ) {
        let rechecks = imports::angular::invalidate(db)?;
        let count = rechecks.len();
        refs.extend(rechecks);
        count
    } else {
        0
    };
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
    let mut statement=sql(db.prepare("SELECT id FROM unresolved_refs WHERE status='pending' AND (reference_name IN(SELECT name FROM nodes WHERE file_path=?1) OR reference_name IN(SELECT qualified_name FROM nodes WHERE file_path=?1) OR ((?2 IN ('typescript','tsx','javascript','jsx') OR ?3=1) AND language IN ('typescript','tsx','javascript','jsx') AND file_path IN(SELECT file_path FROM nodes WHERE kind='import')) OR ((?2 IN ('typescript','tsx','javascript','jsx') OR ?3=1) AND language IN ('typescript','tsx','javascript','jsx') AND reference_kind='angular_injects') OR ((?2 IN ('typescript','tsx','javascript','jsx','vue') OR ?3=1) AND language='vue' AND reference_kind='calls' AND file_path IN(SELECT file_path FROM nodes WHERE kind='import')) OR (?2='rust' AND language='rust' AND reference_name LIKE 'super::%' AND reference_kind IN ('calls','function_ref')) OR (?2='html' AND reference_kind='framework_template' AND reference_name=?1)) LIMIT 20001"))?;
    for row in sql(
        statement.query_map(params![path, language, config_changed], |r| {
            r.get::<_, i64>(0)
        }),
    )? {
        refs.insert(sql(row)?);
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
    framework_snapshot.recheck(project, context)?;
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
    let mut report = json!({"mode":"sync_file","path":path,"changed":true,"extractionPerformed":extracted.is_some(),"deleted":extracted.is_none(),"nodesWritten":node_count,"incomingEdgesRetained":retained_count,"incomingReferencesRecovered":resurrected_count,"providerNodesPreserved":provider_ids.len(),"providerEdgesRetained":provider_kept,"providerEdgesDropped":provider_dropped,"importReferencesRechecked":import_recheck_count,"frameworkReferencesRechecked":framework_rechecks,"razorReferencesRechecked":razor_recheck_count,"referencesAttempted":refs.len(),"referencesResolved":resolved,"referencesPending":refs.len()-resolved,"resolutionPolicy":"exact-name-relative-import-and-framework-subsets","committed":false,"synchronized":false,"globalFreshnessCertified":false});
    if let Some(coverage) = extracted.and_then(|value| value.get("coverage")) {
        report["extractionCoverage"] = coverage.clone();
    }
    Ok((report, import_sources))
}

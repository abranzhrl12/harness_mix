//! SQLite implementation of the application publication session.
use super::*;

#[allow(clippy::too_many_arguments)] // Publication inputs plus the transaction's configuration snapshot and invalidator port.
pub(in crate::adapters::index_store) fn publish(
    db: &Transaction<'_>,
    project: &Project,
    path: &str,
    language: &str,
    bytes: Option<&[u8]>,
    context: &OperationContext,
    configuration: (tsconfig::Snapshot, bool),
    force: bool,
    prepared: Option<super::super::parallel::Prepared>,
    retry_memo: &mut super::super::resolution::RetryMemo,
    invalidator: &ReferenceInvalidator,
) -> Result<(Value, imports::Sources), Failure> {
    let (configuration, config_changed) = configuration;
    let mut session = SqlitePublication {
        db,
        project,
        path,
        language,
        bytes,
        configuration: Some(configuration),
        config_changed,
        force,
        prepared,
        retry_memo,
        invalidator,
    };
    harness_application::indexing::publish(&mut session, context)
}

struct SqlitePublication<'a, 'db> {
    db: &'a Transaction<'db>,
    project: &'a Project,
    path: &'a str,
    language: &'a str,
    bytes: Option<&'a [u8]>,
    configuration: Option<tsconfig::Snapshot>,
    config_changed: bool,
    force: bool,
    prepared: Option<super::super::parallel::Prepared>,
    retry_memo: &'a mut super::super::resolution::RetryMemo,
    invalidator: &'a ReferenceInvalidator,
}

struct CapturedGraph {
    before_declarations: Option<Vec<retry::Declaration>>,
    razor_changes: Option<razor_invalidation::Changes>,
    provider_ids: BTreeSet<String>,
    provider_edges: Vec<ProviderEdge>,
    paired: std::collections::BTreeMap<String, String>,
    retained: Vec<super::super::repository::Incoming>,
    resurrect: Vec<(super::super::repository::Incoming, String, String)>,
    now: i64,
    generated: i64,
}

struct WrittenGraph {
    node_count: usize,
    refs: BTreeSet<i64>,
    provider_kept: usize,
    provider_dropped: usize,
    extraction_coverage: Option<Value>,
}

struct ResolutionResult {
    resolved: usize,
    import_sources: imports::Sources,
    framework_snapshot: framework::Snapshot,
}

impl harness_application::ports::indexing::PublicationSession for SqlitePublication<'_, '_> {
    type Extraction = Option<Value>;
    type Captured = CapturedGraph;
    type Written = WrittenGraph;
    type Invalidation = invalidation::InvalidationOutcome;
    type Resolution = ResolutionResult;
    type Output = (Value, imports::Sources);

    fn stage(&mut self, stage: harness_application::indexing::PublicationStage) {
        sync_diagnostic::stage(stage.as_str());
    }
    fn unchanged(&mut self, _: &OperationContext) -> Result<Option<Self::Output>, Failure> {
        let (db, path, bytes, config_changed, force) = (
            self.db,
            self.path,
            self.bytes,
            self.config_changed,
            self.force,
        );
        let record: Option<(String, Option<String>)> = sql(db
            .query_row(
                "SELECT content_hash,errors FROM files WHERE path=?1",
                [path],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional())?;
        if plan::is_unchanged(record.as_ref(), bytes, config_changed, force, hash) {
            return Ok(Some((
                model::PublicationReport::unchanged(path),
                imports::Sources::configured(self.configuration.take().ok_or(INDEX)?),
            )));
        }
        Ok(None)
    }

    fn extract(&mut self, context: &OperationContext) -> Result<Self::Extraction, Failure> {
        let (bytes, path) = (self.bytes, self.path);
        let mut prepared = self.prepared.take();
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
        Ok(extracted_value)
    }

    fn capture(
        &mut self,
        extraction: &Self::Extraction,
        context: &OperationContext,
    ) -> Result<Self::Captured, Failure> {
        let (db, path, language) = (self.db, self.path, self.language);
        let retry_memo = &self.retry_memo;
        let extracted = extraction.as_ref();
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
        Ok(CapturedGraph {
            before_declarations,
            razor_changes,
            provider_ids,
            provider_edges,
            paired,
            retained,
            resurrect,
            now,
            generated,
        })
    }

    fn remove_previous(&mut self, _: &Self::Captured, _: &OperationContext) -> Result<(), Failure> {
        let (db, path) = (self.db, self.path);
        sql(db.execute("DELETE FROM unresolved_refs WHERE from_node_id IN(SELECT id FROM nodes WHERE file_path=?1 AND (kind NOT IN('route','component') OR id LIKE 'native:framework:%'))",[path]))?;
        sql(db.execute("DELETE FROM edges WHERE source IN(SELECT id FROM nodes WHERE file_path=?1) OR target IN(SELECT id FROM nodes WHERE file_path=?1)",[path]))?;
        sql(db.execute(
        "DELETE FROM nodes WHERE file_path=?1 AND (kind NOT IN('route','component') OR id LIKE 'native:framework:%')",
        [path],
    ))?;
        Ok(())
    }

    fn write(
        &mut self,
        extraction: &Self::Extraction,
        captured: &Self::Captured,
        context: &OperationContext,
    ) -> Result<Self::Written, Failure> {
        let (db, path, language, bytes) = (self.db, self.path, self.language, self.bytes);
        let (now, generated, provider_ids) =
            (captured.now, captured.generated, &captured.provider_ids);
        let extracted = extraction.as_ref();
        let prepared_graph = extracted
            .map(|value| model::ExtractedGraph::prepare(value, context))
            .transpose()?;
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
        Ok(WrittenGraph {
            node_count: graph_write.node_count,
            refs: graph_write.refs,
            provider_kept: 0,
            provider_dropped: 0,
            extraction_coverage: prepared_graph
                .as_ref()
                .and_then(|graph| graph.coverage)
                .cloned(),
        })
    }

    fn restore(
        &mut self,
        captured: &Self::Captured,
        written: &mut Self::Written,
        context: &OperationContext,
    ) -> Result<(), Failure> {
        let (db, path) = (self.db, self.path);
        let CapturedGraph {
            retained,
            resurrect,
            provider_edges,
            provider_ids,
            paired,
            ..
        } = captured;
        let refs = &mut written.refs;
        batches::insert(
            db,
            "INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES ",
            7,
            retained,
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
        for edge in provider_edges {
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

        written.provider_kept = provider_kept;
        written.provider_dropped = provider_dropped;
        Ok(())
    }

    fn invalidate(
        &mut self,
        captured: &Self::Captured,
        written: &mut Self::Written,
        context: &OperationContext,
    ) -> Result<Self::Invalidation, Failure> {
        let (db, path, language, config_changed) =
            (self.db, self.path, self.language, self.config_changed);
        let (before_declarations, razor_changes) =
            (&captured.before_declarations, &captured.razor_changes);
        let mut invalidation = self.invalidator.invalidate(
            db,
            context,
            InvalidationScope {
                path,
                language,
                config_changed,
                before_declarations,
                razor_changes,
                retry_memo: self.retry_memo,
            },
        )?;
        written.refs.append(&mut invalidation.recheck_refs);
        if written.refs.len() > MAX_ROWS {
            return Err(LIMIT);
        }
        Ok(invalidation)
    }

    fn resolve(
        &mut self,
        written: &Self::Written,
        invalidation: &Self::Invalidation,
        context: &OperationContext,
    ) -> Result<Self::Resolution, Failure> {
        let (db, project, language) = (self.db, self.project, self.language);
        let refs = &written.refs;
        let after_declarations = &invalidation.after_declarations;
        let retry_memo = &mut self.retry_memo;
        let mut candidate_snapshot = candidates::Snapshot::default();
        let mut resolved = 0;
        let mut import_sources =
            imports::Sources::configured(self.configuration.take().ok_or(INDEX)?);
        let mut mvc_snapshot = super::super::super::index_razor_mvc::Snapshot::default();
        let mut framework_snapshot = framework::Snapshot::default();
        for id in refs {
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
            && let Some(declarations) = after_declarations
        {
            retry_memo.remember(retry::visible_names(declarations));
        }
        Ok(ResolutionResult {
            resolved,
            import_sources,
            framework_snapshot,
        })
    }

    fn recheck(
        &mut self,
        resolution: &Self::Resolution,
        context: &OperationContext,
    ) -> Result<(), Failure> {
        resolution.framework_snapshot.recheck(self.project, context)
    }

    fn metadata(
        &mut self,
        captured: &Self::Captured,
        resolution: &Self::Resolution,
        _: &OperationContext,
    ) -> Result<(), Failure> {
        let (db, now, import_sources) = (self.db, captured.now, &resolution.import_sources);
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
        Ok(())
    }

    fn finish(
        &mut self,
        extraction: Self::Extraction,
        captured: Self::Captured,
        written: Self::Written,
        invalidation: Self::Invalidation,
        resolution: Self::Resolution,
    ) -> Result<Self::Output, Failure> {
        let path = self.path;
        let config_changed = self.config_changed;
        let retry_memo = &self.retry_memo;
        let CapturedGraph {
            provider_ids,
            retained,
            resurrect,
            ..
        } = captured;
        let WrittenGraph {
            node_count,
            refs,
            provider_kept,
            provider_dropped,
            extraction_coverage,
        } = written;
        let ResolutionResult {
            resolved,
            import_sources,
            ..
        } = resolution;
        let invalidation::InvalidationOutcome {
            import_recheck_count,
            framework_recheck_count,
            razor_recheck_count,
            declaration_pool_changed,
            ..
        } = invalidation;
        let extraction_performed = extraction.is_some();
        let retained_count = retained.len();
        let resurrected_count = resurrect.len();
        let extracted = extraction.as_ref();
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
            extraction_coverage,
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
}

use super::{candidates, imports, razor_invalidation, sql};
use crate::adapters::project::Project;
use harness_domain::{Failure, execution::OperationContext};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

type ResolutionReference = (String, String, String, i64, i64, String, String, String);

/// Operation-local retry coverage for an unchanged declaration snapshot.
/// A name is retained only after its pending refs were considered. Changing
/// declarations, a competing commit, or rollback invalidates the coverage.
pub(super) struct RetryMemo {
    names: std::collections::BTreeSet<String>,
    bytes: usize,
    data_version: Option<i64>,
    enabled: bool,
}

impl Default for RetryMemo {
    fn default() -> Self {
        Self {
            names: Default::default(),
            bytes: 0,
            data_version: None,
            enabled: std::env::var("HARNESS_INDEX_RESOLUTION_MEMO").as_deref() != Ok("0"),
        }
    }
}

impl RetryMemo {
    pub(super) fn clear(&mut self) {
        self.names.clear();
        self.bytes = 0;
    }
    pub(super) fn observe_version(&mut self, version: i64) {
        if self.data_version != Some(version) {
            self.clear();
            self.data_version = Some(version);
        }
    }
    pub(super) fn enabled(&self) -> bool {
        self.enabled
    }
    pub(super) fn retained_names(&self) -> usize { self.names.len() }
    pub(super) fn data_version(&self) -> Option<i64> { self.data_version }
    pub(super) fn json(&self) -> String {
        serde_json::to_string(&self.names).expect("string set is JSON serializable")
    }
    pub(super) fn remember(&mut self, names: impl IntoIterator<Item = String>) {
        if !self.enabled {
            return;
        }
        for name in names {
            if self.names.contains(&name) {
                continue;
            }
            if self.names.len() < 4096 && self.bytes + name.len() <= 65536 {
                self.bytes += name.len();
                self.names.insert(name);
            }
        }
    }
}

#[cfg(test)]
#[path = "resolution_tests.rs"]
mod retry_memo_tests;
#[path = "framework_resolution.rs"]
mod framework;
pub(super) fn resolve_exact(
    db: &Connection,
    id: i64,
    project: &Project,
    context: &OperationContext,
    sources: &mut imports::Sources,
    candidate_snapshot: &mut candidates::Snapshot,
    mvc_snapshot: &mut super::super::index_razor_mvc::Snapshot,
    framework_snapshot: &mut super::repository::framework::Snapshot,
) -> Result<bool, Failure> {
    let reference:Option<ResolutionReference> = sql(sql(db.prepare_cached("SELECT r.from_node_id,r.reference_name,r.reference_kind,r.line,r.col,r.file_path,r.language,n.qualified_name FROM unresolved_refs r JOIN nodes n ON n.id=r.from_node_id WHERE r.id=?1"))?.query_row([id],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional())?;
    let Some((from, name, kind, line, col, path, language, owner)) = reference else {
        return Ok(false);
    };
    if matches!(
        kind.as_str(),
        "angular_route_component" | "angular_route_layout"
    ) {
        return framework::angular_route_component(
            db,
            id,
            project,
            context,
            sources,
            framework::Site {
                from: &from,
                name: &name,
                kind: &kind,
                line,
                col,
                path: &path,
            },
        );
    }
    if matches!(
        kind.as_str(),
        "angular_provider_token" | "angular_provider_implementation"
    ) {
        return framework::angular_provider(
            db,
            id,
            project,
            context,
            sources,
            framework::Site {
                from: &from,
                name: &name,
                kind: &kind,
                line,
                col,
                path: &path,
            },
        );
    }
    if super::repository::framework::is_template_reference(&kind) {
        let Some(target) = super::repository::framework::indexed_template_target(
            db, project, &from, &path, &language, &name, context,
        )?
        else {
            // Missing, stale, ambiguous, or unsafe paths stay pending. Never
            // fall through to the generic same-name resolver.
            return Ok(false);
        };
        framework_snapshot.remember(&path, &target.source_hash)?;
        framework_snapshot.remember(&target.target_path, &target.target_hash)?;
        let metadata = json!({
            "refName": name,
            "refKind": kind,
            "resolvedBy": "native-angular-template-url-indexed-html-v1",
            "confidence": 1.0,
            "sourceHash": target.source_hash,
            "targetHash": target.target_hash
        })
        .to_string();
        sql(db.execute(
            "INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES(?1,?2,'references',?3,?4,?5,'heuristic')",
            params![from, target.node_id, line, col, metadata],
        ))?;
        sql(db.execute("DELETE FROM unresolved_refs WHERE id=?1", [id]))?;
        return Ok(true);
    }
    if matches!(language.as_str(), "vue" | "html") {
        if language == "vue" && kind == "calls" {
            let Some(target) =
                sources.vue_call_target(db, project, context, &path, &name, line, col)?
            else {
                // Returned methods have an independent lexical/data-flow proof.
                // Neither route may fall through to global same-name matching.
                return framework::vue_method(
                    db,
                    id,
                    project,
                    context,
                    sources,
                    framework::Site {
                        from: &from,
                        name: &name,
                        kind: &kind,
                        line,
                        col,
                        path: &path,
                    },
                );
            };
            let mut statement = sql(db.prepare_cached(
                "SELECT id FROM nodes WHERE file_path=?1 AND name=?2 COLLATE BINARY AND kind='function' AND language IN ('typescript','tsx','javascript','jsx') LIMIT 2",
            ))?;
            let ids = sql(
                statement.query_map(params![target.path, target.name], |row| {
                    row.get::<_, String>(0)
                }),
            )?
            .collect::<Result<Vec<_>, _>>();
            let ids = sql(ids)?;
            if ids.len() != 1 {
                return Ok(false);
            }
            let mut metadata = json!({
                "refName": name,
                "refKind": kind,
                "resolvedBy": "vue-named-composable-subset-v1",
                "confidence": 0.9,
                "exportContributors": target.contributors,
                "bindingEngine": "oxc-semantic-vue-sfc-subset-v1",
                "moduleEngine": "oxc-resolver-11.0.0-index-snapshot",
                "exportEngine": "oxc-module-record-runtime-resolve-v1"
            });
            if sources.config.path.is_some() {
                metadata["tsconfig"] = json!(sources.config.stamp());
            }
            let metadata = metadata.to_string();
            sql(db.execute(
                "INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES(?1,?2,'calls',?3,?4,?5,'heuristic')",
                params![from, ids[0], line, col, metadata],
            ))?;
            sql(db.execute("DELETE FROM unresolved_refs WHERE id=?1", [id]))?;
            return Ok(true);
        }
        // HTML bindings and unsupported Vue reference kinds remain pending.
        return Ok(false);
    }
    if kind == "angular_injects" {
        let Some(proof) =
            sources.angular_token_target(db, project, context, &path, &name, line, col)?
        else {
            // The persisted extraction candidates are not authority. Only a
            // fresh AST/import binding proof can establish a token dependency.
            return Ok(false);
        };
        let mut statement = sql(db.prepare_cached(
            "SELECT id FROM nodes WHERE file_path=?1 AND name=?2 COLLATE BINARY AND kind='class' AND language IN ('typescript','tsx','javascript','jsx') LIMIT 2",
        ))?;
        let ids = sql(
            statement.query_map(params![proof.target.path, proof.target.name], |row| {
                row.get::<_, String>(0)
            }),
        )?
        .collect::<Result<Vec<_>, _>>();
        let ids = sql(ids)?;
        if ids.len() != 1 {
            return Ok(false);
        }
        let mut metadata = json!({
            "refName": name,
            "refKind": kind,
            "resolvedBy": "angular-token-dependency-subset-v1",
            "confidence": 0.9,
            "framework": "angular",
            "module": proof.module,
            "imported": proof.imported,
            "tokenForm": proof.token_form,
            "exportContributors": proof.target.contributors,
            "bindingEngine": "oxc-angular-di-proof-subset-v1",
            "moduleEngine": "oxc-resolver-11.0.0-index-snapshot",
            "exportEngine": "oxc-module-record-runtime-resolve-v1"
        });
        if sources.config.path.is_some() {
            metadata["tsconfig"] = json!(sources.config.stamp());
        }
        let metadata = metadata.to_string();
        sql(db.execute(
            "INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES(?1,?2,'references',?3,?4,?5,'heuristic')",
            params![from, ids[0], line, col, metadata],
        ))?;
        sql(db.execute("DELETE FROM unresolved_refs WHERE id=?1", [id]))?;
        return Ok(true);
    }
    if language == "razor" && kind == "mvc_route" {
        let Some(target) = mvc_snapshot.resolve(db, &name, &path, context)? else {
            razor_invalidation::mark_failed(db, id, &name, &kind)?;
            return Ok(false);
        };
        let metadata = json!({"refName":name,"refKind":kind,
            "resolvedBy":"razor-mvc-literal-route-subset-v1",
            "confidence":0.9})
        .to_string();
        sql(db.execute("INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES(?1,?2,'calls',?3,?4,?5,'heuristic')", params![from,target,line,col,metadata]))?;
        sql(db.execute("DELETE FROM unresolved_refs WHERE id=?1", [id]))?;
        return Ok(true);
    }
    if language == "razor" && matches!(kind.as_str(), "razor_partial" | "razor_layout") {
        let Some(target) = mvc_snapshot.resolve_view(db, &name, &path, context)? else {
            razor_invalidation::mark_failed(db, id, &name, &kind)?;
            return Ok(false);
        };
        let metadata = json!({"refName":name,"refKind":kind,
            "resolvedBy":"razor-view-target-subset-v1",
            "confidence":0.9})
        .to_string();
        sql(db.execute("INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES(?1,?2,'references',?3,?4,?5,'heuristic')", params![from,target,line,col,metadata]))?;
        sql(db.execute("DELETE FROM unresolved_refs WHERE id=?1", [id]))?;
        return Ok(true);
    }
    if language == "razor" && kind == "calls" {
        // The tail index permits exact affected-name retries after C# edits.
        razor_invalidation::mark_failed(db, id, &name, &kind)?;
    }
    let allowed: &[&str] = match kind.as_str() {
        "calls" | "function_ref" => &["function", "method"],
        "instantiates" => &["class", "struct"],
        "extends" | "implements" => &["class", "struct", "interface", "trait"],
        "references" if language == "razor" => &["class", "struct", "record", "interface", "enum"],
        _ => return Ok(false),
    };
    let js = matches!(
        language.as_str(),
        "typescript" | "tsx" | "javascript" | "jsx"
    );
    if js
        && sources
            .config
            .semantic
            .as_ref()
            .is_some_and(|policy| policy.excluded(&path))
    {
        return Ok(false);
    }
    let mut scope = if js {
        imports::scope(db, &path, &name, &language, context, sources)?
    } else {
        imports::Scope::None
    };
    if matches!(scope, imports::Scope::Pending) {
        return Ok(false);
    }
    let mut export_contributors = Vec::new();
    if let imports::Scope::Binding {
        path: target,
        name: exported,
    } = &scope
    {
        if kind != "calls" || !sources.permits(db, project, context, &path, &name, line, col)? {
            return Ok(false);
        }
        let Some(target) = sources.export_target(db, project, context, target, exported)? else {
            return Ok(false);
        };
        export_contributors = target.contributors;
        scope = imports::Scope::Binding {
            path: target.path,
            name: target.name,
        };
    }
    let (lookup, imported_path) = match &scope {
        imports::Scope::Binding { path, name } => (name.as_str(), Some(path.as_str())),
        _ => (name.as_str(), None),
    };
    // The C# kernel qualifies the namespace/type boundary with `::`; Razor's
    // @model directive uses C# source spelling (`Demo.Model`).
    let model_lookup = if language == "razor" && kind == "references" {
        lookup
            .rsplit_once('.')
            .map(|(namespace, name)| format!("{namespace}::{name}"))
    } else {
        None
    };
    let lookup = model_lookup.as_deref().unwrap_or(lookup);
    // Receivers/chains are never stripped. Named imports override global names;
    // unsupported import forms defer instead of falling back to a homonym.
    let rust_target = if language == "rust" && matches!(kind.as_str(), "calls" | "function_ref") {
        let Some(target) = sources.rust_target(db, project, context, &path, &name, line, col)?
        else {
            return Ok(false);
        };
        Some(target)
    } else {
        None
    };
    // An absolute C# type receiver cannot be a local variable or a Razor
    // member. Resolve only unique public static methods; instance receivers,
    // aliases, unqualified names and overloads still need semantic binding.
    let razor_static_target = if language == "razor" && kind == "calls" {
        if let Some(absolute) = name.strip_prefix("global::") {
            if absolute.split('.').count() >= 2
                && absolute.split('.').all(|part| {
                    !part.is_empty()
                        && part.bytes().enumerate().all(|(i, b)| {
                            b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit())
                        })
                })
            {
                let mut statement = sql(db.prepare_cached(
                    "SELECT id FROM nodes WHERE name=?2 COLLATE BINARY AND language='csharp' AND kind='method' AND visibility='public' AND is_static=1 AND replace(qualified_name,'::','.')=?1 COLLATE BINARY LIMIT 2",
                ))?;
                let matches = sql(statement.query_map(
                    [absolute, absolute.rsplit('.').next().unwrap_or("")],
                    |row| row.get::<_, String>(0),
                ))?
                .collect::<Result<Vec<_>, _>>();
                let matches = sql(matches)?;
                (matches.len() == 1).then(|| matches[0].clone())
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    let razor_instance_target = if language == "razor"
        && kind == "calls"
        && razor_static_target.is_none()
        && name.contains('.')
    {
        sources.razor_instance_target(db, project, context, &path, &name, line, col)?
    } else {
        None
    };
    let mut candidates = if let Some((target, _)) = &razor_instance_target {
        vec![sql(db.query_row("SELECT id,kind,file_path,qualified_name,visibility,is_exported,language FROM nodes WHERE id=?1", [target], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))))?]
    } else if let Some(target) = &rust_target {
        vec![sql(db.query_row("SELECT id,kind,file_path,qualified_name,visibility,is_exported,language FROM nodes WHERE id=?1", [target], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))))?]
    } else if let Some(target) = &razor_static_target {
        vec![sql(db.query_row("SELECT id,kind,file_path,qualified_name,visibility,is_exported,language FROM nodes WHERE id=?1", [target], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))))?]
    } else {
        candidate_snapshot.lookup(
            db,
            lookup,
            if language == "razor" && kind == "references" {
                "csharp"
            } else {
                &language
            },
            imported_path,
            context,
        )?
    };
    if candidates.len() > 64 {
        return Ok(false);
    }
    candidates.retain(|n| allowed.contains(&n.1.as_str()));
    // Member blocks belong to the generated class of this view. A homonymous
    // method in another view is never proof of a Razor call target.
    if language == "razor"
        && matches!(kind.as_str(), "calls" | "function_ref")
        && razor_static_target.is_none()
        && razor_instance_target.is_none()
    {
        candidates.retain(|n| n.2 == path);
    }
    if let Some(target) = imported_path {
        candidates.retain(|n| n.2 == target && n.1 == "function" && n.3 == lookup);
    } else if matches!(scope, imports::Scope::LocalOnly) {
        candidates.retain(|n| n.2 == path);
    }
    // Cross-file private symbols require receiver/import/visibility analysis
    // outside this subset. Keep them pending rather than guessing accessibility.
    if imported_path.is_none() && rust_target.is_none() {
        candidates.retain(|n| n.2 == path || n.4.as_deref() == Some("public") || n.5);
    }
    let qualified = lookup.contains('.') || lookup.contains("::");
    if rust_target.is_some() || razor_static_target.is_some() || razor_instance_target.is_some() {
        // The language-specific proof already selected the exact declaration.
    } else if qualified {
        candidates.retain(|n| n.3 == lookup);
    } else if imported_path.is_none() {
        let container = owner
            .rsplit_once("::")
            .map(|(p, _)| format!("{p}::"))
            .or_else(|| owner.rsplit_once('.').map(|(p, _)| format!("{p}.")));
        if let Some(container) = container {
            let scoped = candidates
                .iter()
                .filter(|n| n.2 == path && n.3 == format!("{container}{name}"))
                .cloned()
                .collect::<Vec<_>>();
            if !scoped.is_empty() {
                candidates = scoped;
            }
        }
        let local = candidates
            .iter()
            .filter(|n| n.2 == path)
            .cloned()
            .collect::<Vec<_>>();
        if !local.is_empty() {
            candidates = local;
        }
    }
    if candidates.len() != 1 {
        return Ok(false);
    }
    let edge_kind = if kind == "function_ref" {
        "references"
    } else {
        &kind
    };
    let mut metadata = json!({"refName":name,"refKind":kind,"resolvedBy":if imported_path.is_some(){"native-relative-named-import-subset"}else{"native-exact-name-subset"},"confidence":0.9});
    if razor_static_target.is_some() {
        metadata["resolvedBy"] = json!("razor-absolute-static-method-subset-v1");
    }
    if let Some((_, receiver_type)) = &razor_instance_target {
        metadata["resolvedBy"] = json!("razor-typed-instance-method-subset-v1");
        metadata["receiverType"] = json!(
            receiver_type
                .strip_prefix("global::")
                .unwrap_or(receiver_type)
        );
    }
    if rust_target.is_some() {
        metadata["resolvedBy"] = json!(if candidates[0].2 == path {
            "ra-lexical-subset-v1"
        } else {
            "ra-module-subset-v1"
        });
    }
    if imported_path.is_some() {
        if sources.config.path.is_some() {
            metadata["tsconfig"] = sources.config.stamp();
        }
        metadata["bindingEngine"] = json!("oxc-semantic-0.90.0");
        metadata["moduleEngine"] = json!("oxc-resolver-11.0.0-index-snapshot");
        metadata["exportEngine"] = json!("oxc-module-record-0.90.0-resolve-export-v2");
        metadata["exportContributors"] = json!(export_contributors);
    }
    let metadata = metadata.to_string();
    sql(sql(db.prepare_cached("INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES(?1,?2,?3,?4,?5,?6,'heuristic')"))?.execute(params![from,candidates[0].0,edge_kind,line,col,metadata]))?;
    sql(sql(db.prepare_cached("DELETE FROM unresolved_refs WHERE id=?1"))?.execute([id]))?;
    Ok(true)
}

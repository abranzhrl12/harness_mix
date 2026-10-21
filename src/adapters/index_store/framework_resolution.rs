//! Publication of source-proven framework relationships, never injector selection.
use super::super::{imports, sql};
use crate::adapters::project::Project;
use harness_domain::{Failure, execution::OperationContext};
use rusqlite::{Connection, params};
use serde_json::json;

pub(super) struct Site<'a> {
    pub from: &'a str,
    pub name: &'a str,
    pub kind: &'a str,
    pub line: i64,
    pub col: i64,
    pub path: &'a str,
}

pub(super) fn angular_provider(
    db: &Connection,
    id: i64,
    project: &Project,
    context: &OperationContext,
    sources: &mut imports::Sources,
    site: Site<'_>,
) -> Result<bool, Failure> {
    let Some(proof) = sources.angular_provider_target(
        db, project, context, site.path, site.name, site.line, site.col, site.kind,
    )?
    else {
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
        "refName": site.name, "refKind": site.kind,
        "resolvedBy": "angular-static-provider-registration-subset-v1",
        "framework": "angular", "relationship": proof.relationship,
        "registrationForm": proof.registration_form,
        "module": proof.module, "imported": proof.imported,
        "runtimeProviderSelectionEvaluated": false,
        "exportContributors": proof.target.contributors,
        "bindingEngine": "oxc-angular-static-provider-proof-subset-v1"
    });
    if sources.config.path.is_some() {
        metadata["tsconfig"] = json!(sources.config.stamp());
    }
    sql(db.execute(
        "INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES(?1,?2,'references',?3,?4,?5,'heuristic')",
        params![site.from, ids[0], site.line, site.col, metadata.to_string()],
    ))?;
    sql(db.execute("DELETE FROM unresolved_refs WHERE id=?1", [id]))?;
    Ok(true)
}

pub(super) fn vue_method(
    db: &Connection,
    id: i64,
    project: &Project,
    context: &OperationContext,
    sources: &mut imports::Sources,
    site: Site<'_>,
) -> Result<bool, Failure> {
    let Some(proof) = sources.vue_returned_method_target(
        db, project, context, site.path, site.name, site.line, site.col,
    )?
    else {
        return Ok(false);
    };
    let mut statement = sql(db.prepare_cached(
        "SELECT id FROM nodes WHERE file_path=?1 AND name=?2 COLLATE BINARY AND kind='function' AND start_line=?3 AND start_column=?4 AND language IN ('typescript','tsx','javascript','jsx') LIMIT 2",
    ))?;
    let ids = sql(statement.query_map(
        params![
            proof.target.path,
            proof.implementation.name,
            proof.implementation.line,
            proof.implementation.col
        ],
        |row| row.get::<_, String>(0),
    ))?
    .collect::<Result<Vec<_>, _>>();
    let ids = sql(ids)?;
    if ids.len() != 1 {
        return Ok(false);
    }
    let mut metadata = json!({
        "refName": site.name, "refKind": site.kind,
        "resolvedBy": "vue-composable-return-method-subset-v1",
        "framework": "vue", "composable": proof.target.name,
        "exportContributors": proof.target.contributors,
        "bindingEngine": "oxc-vue-returned-method-proof-subset-v1"
    });
    if sources.config.path.is_some() {
        metadata["tsconfig"] = json!(sources.config.stamp());
    }
    sql(db.execute(
        "INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES(?1,?2,'calls',?3,?4,?5,'heuristic')",
        params![site.from, ids[0], site.line, site.col, metadata.to_string()],
    ))?;
    sql(db.execute("DELETE FROM unresolved_refs WHERE id=?1", [id]))?;
    Ok(true)
}

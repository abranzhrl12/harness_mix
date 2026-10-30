use super::{sql, sync_diagnostic};
use harness_domain::Failure;
use rusqlite::{Connection, OptionalExtension};
use serde_json::json;

fn normalized(sql: &str) -> String {
    sql.to_ascii_lowercase()
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != ';')
        .collect::<String>()
        .replace("ifnotexists", "")
}

#[cfg(test)]
#[path = "validation_tests.rs"]
mod tests;

pub(super) fn validate_schema(db: &Connection) -> Result<(), Failure> {
    sync_diagnostic::schema_check("required_tables_catalog_query");
    const REQUIRED_TABLES: [&str; 6] = [
        "schema_versions",
        "nodes",
        "edges",
        "files",
        "unresolved_refs",
        "project_metadata",
    ];
    let mut statement = sql(db.prepare(
        "SELECT name, type, sql FROM sqlite_schema WHERE name IN ('schema_versions','nodes','edges','files','unresolved_refs','project_metadata')",
    ))?;
    let rows = sql(statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    }))?;
    sync_diagnostic::schema_check("required_tables_catalog_rows");
    let mut valid_tables = Vec::new();
    let mut invalid_tables = Vec::new();
    for row in rows {
        let (name, kind, definition) = sql(row)?;
        let is_plain_table = kind == "table"
            && definition.as_deref().is_some_and(|sql| {
                sql.trim_start()
                    .to_ascii_uppercase()
                    .starts_with("CREATE TABLE")
            });
        if is_plain_table {
            valid_tables.push(name);
        } else {
            invalid_tables.push(name);
        }
    }
    let missing_tables: Vec<&str> = REQUIRED_TABLES
        .iter()
        .copied()
        .filter(|required| !valid_tables.iter().any(|found| found == required))
        .collect();
    if !missing_tables.is_empty() || !invalid_tables.is_empty() {
        sync_diagnostic::schema_issue(json!({
            "check": "required_tables",
            "expected": REQUIRED_TABLES.len(),
            "found": valid_tables.len(),
            "missing": missing_tables,
            "notOrdinaryTables": invalid_tables
        }));
        return Err((
            "SYNC_SCHEMA",
            "Required CodeGraph tables are missing or are not ordinary tables; see diagnostic.schemaValidation",
        ));
    }
    sync_diagnostic::schema_check("provider_schema_version_query");
    let version: i64 =
        sql(db.query_row("SELECT max(version) FROM schema_versions", [], |r| r.get(0)))?;
    if version != 11 {
        sync_diagnostic::schema_issue(json!({
            "check": "provider_schema_version",
            "expected": 11,
            "actual": version
        }));
        return Err((
            "SYNC_SCHEMA",
            "CodeGraph provider schema version mismatch; see diagnostic.schemaValidation",
        ));
    }
    sync_diagnostic::schema_check("trigger_catalog_query");
    let mut statement =
        sql(db.prepare("SELECT name,sql FROM sqlite_schema WHERE type='trigger' LIMIT 4"))?;
    let rows = sql(statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }))?;
    sync_diagnostic::schema_check("trigger_catalog_rows");
    let mut trigger_count = 0;
    for row in rows {
        let (name, body) = sql(row)?;
        let expected = match name.as_str() {
            "nodes_ai" => {
                "CREATE TRIGGER nodes_ai AFTER INSERT ON nodes BEGIN INSERT INTO nodes_fts(rowid,id,name,qualified_name,docstring,signature) VALUES(NEW.rowid,NEW.id,NEW.name,NEW.qualified_name,NEW.docstring,NEW.signature); END"
            }
            "nodes_ad" => {
                "CREATE TRIGGER nodes_ad AFTER DELETE ON nodes BEGIN INSERT INTO nodes_fts(nodes_fts,rowid,id,name,qualified_name,docstring,signature) VALUES('delete',OLD.rowid,OLD.id,OLD.name,OLD.qualified_name,OLD.docstring,OLD.signature); END"
            }
            "nodes_au" => {
                "CREATE TRIGGER nodes_au AFTER UPDATE ON nodes BEGIN INSERT INTO nodes_fts(nodes_fts,rowid,id,name,qualified_name,docstring,signature) VALUES('delete',OLD.rowid,OLD.id,OLD.name,OLD.qualified_name,OLD.docstring,OLD.signature); INSERT INTO nodes_fts(rowid,id,name,qualified_name,docstring,signature) VALUES(NEW.rowid,NEW.id,NEW.name,NEW.qualified_name,NEW.docstring,NEW.signature); END"
            }
            _ => {
                return Err((
                    "SYNC_SCHEMA",
                    "Unrecognized index trigger; publication refused",
                ));
            }
        };
        if normalized(&body) != normalized(expected) {
            return Err(("SYNC_SCHEMA", "Modified index trigger; publication refused"));
        }
        trigger_count += 1;
    }
    if trigger_count != 3 {
        sync_diagnostic::schema_issue(
            json!({"check":"trigger_count","expected":3,"actual":trigger_count}),
        );
        return Err(("SYNC_SCHEMA", "Provider full-text triggers are missing"));
    }
    // Refuse virtual/view substitutions of writable tables via the ordinary-table
    // check above. If FTS exists, require the provider's exact external-content ABI.
    sync_diagnostic::schema_check("fts_definition_query");
    let fts: Option<String> = sql(db
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name='nodes_fts'",
            [],
            |r| r.get(0),
        )
        .optional())?;
    if let Some(fts) = fts {
        let expected = "CREATE VIRTUAL TABLE nodes_fts USING fts5(id,name,qualified_name,docstring,signature,content='nodes',content_rowid='rowid')";
        if normalized(&fts) != normalized(expected) {
            sync_diagnostic::schema_issue(
                json!({"check":"fts_definition","actual":fts,"expected":expected}),
            );
            return Err(("SYNC_SCHEMA", "Unsupported full-text index definition"));
        }
    } else {
        sync_diagnostic::schema_issue(
            json!({"check":"fts_definition","actual":null,"expected":"provider FTS5 external-content table"}),
        );
        return Err(("SYNC_SCHEMA", "Provider full-text index is missing"));
    }
    sync_diagnostic::schema_validated();
    Ok(())
}

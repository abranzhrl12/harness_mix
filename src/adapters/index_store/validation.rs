use super::{INDEX, sql};
use harness_domain::Failure;
use rusqlite::{Connection, OptionalExtension};

fn normalized(sql: &str) -> String {
    sql.to_ascii_lowercase()
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != ';')
        .collect::<String>()
        .replace("ifnotexists", "")
}
pub(super) fn validate_schema(db: &Connection) -> Result<(), Failure> {
    let ordinary: i64 = sql(db.query_row("SELECT count(*) FROM sqlite_schema WHERE name IN ('schema_versions','nodes','edges','files','unresolved_refs','project_metadata') AND type='table' AND upper(sql) LIKE 'CREATE TABLE%'", [], |r| r.get(0)))?;
    if ordinary != 6 {
        return Err(INDEX);
    }
    let version: i64 =
        sql(db.query_row("SELECT max(version) FROM schema_versions", [], |r| r.get(0)))?;
    if version != 11 {
        return Err(INDEX);
    }
    let mut statement =
        sql(db.prepare("SELECT name,sql FROM sqlite_schema WHERE type='trigger' LIMIT 4"))?;
    let rows = sql(statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }))?;
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
        return Err(("SYNC_SCHEMA", "Provider full-text triggers are missing"));
    }
    // Refuse virtual/view substitutions of writable tables via the ordinary-table
    // check above. If FTS exists, require the provider's exact external-content ABI.
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
            return Err(("SYNC_SCHEMA", "Unsupported full-text index definition"));
        }
    } else {
        return Err(("SYNC_SCHEMA", "Provider full-text index is missing"));
    }
    Ok(())
}

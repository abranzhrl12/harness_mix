use super::{LIMIT, MAX_ROWS, array, opt_string, sql, string};
use harness_domain::Failure;
use rusqlite::{Connection, params};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub(super) struct Identity {
    pub(super) id: String,
    pub(super) key: String,
}
pub(super) fn identities(db: &Connection, path: &str) -> Result<Vec<Identity>, Failure> {
    let mut statement = sql(db.prepare("SELECT id,kind,qualified_name,ifnull(signature,'') FROM nodes WHERE file_path=?1 LIMIT 20001"))?;
    let rows = sql(statement.query_map([path], |r| {
        Ok(Identity {
            id: r.get(0)?,
            key: format!(
                "{}\0{}\0{}",
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?
            ),
        })
    }))?;
    let mut result = Vec::new();
    let mut reserved = 0usize;
    for row in rows {
        let row = sql(row)?;
        reserved += row.id.len() + row.key.len();
        if result.len() >= MAX_ROWS || reserved > 16 * 1024 * 1024 {
            return Err(LIMIT);
        }
        result.push(row);
    }
    Ok(result)
}
pub(super) fn identity_map(
    prior: Vec<Identity>,
    extracted: Option<&Value>,
) -> Result<BTreeMap<String, String>, Failure> {
    let mut before: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut after: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for node in prior {
        before.entry(node.key).or_default().push(node.id);
    }
    if let Some(value) = extracted {
        for node in array(value, "nodes")? {
            let key = format!(
                "{}\0{}\0{}",
                string(node, "kind")?,
                string(node, "qualifiedName")?,
                opt_string(node, "signature").unwrap_or("")
            );
            after.entry(key).or_default().push(string(node, "id")?);
        }
    }
    let mut result = BTreeMap::new();
    for (key, ids) in before {
        if ids.len() == 1
            && let Some(next) = after.get(&key)
            && next.len() == 1
        {
            result.insert(ids[0].clone(), next[0].clone());
        }
    }
    Ok(result)
}

#[derive(Clone)]
pub(super) struct ProviderEdge {
    pub(super) source: String,
    pub(super) target: String,
    pub(super) kind: String,
    pub(super) line: Option<i64>,
    pub(super) col: Option<i64>,
    pub(super) metadata: Option<String>,
    pub(super) provenance: Option<String>,
    pub(super) source_path: String,
    pub(super) target_path: String,
}
/// Provider-owned framework nodes of one file. The native extractor never
/// produces them, so the publication keeps them instead of dropping them.
pub(super) fn provider_node_ids(db: &Connection, path: &str) -> Result<BTreeSet<String>, Failure> {
    let mut statement = sql(db.prepare(
        "SELECT id FROM nodes WHERE file_path=?1 AND kind IN('route','component') LIMIT 20001",
    ))?;
    let rows = sql(statement.query_map([path], |r| r.get::<_, String>(0)))?;
    let mut result = BTreeSet::new();
    for row in rows {
        let row = sql(row)?;
        if result.len() >= MAX_ROWS {
            return Err(LIMIT);
        }
        result.insert(row);
    }
    Ok(result)
}
/// Edges incident to the provider nodes of one file, with the source file of
/// every edge so the publication can tell provider-owned edges from the
/// incoming edges it already retains.
pub(super) fn provider_edges(db: &Connection, path: &str) -> Result<Vec<ProviderEdge>, Failure> {
    let mut statement = sql(db.prepare(
        "SELECT e.source,e.target,e.kind,e.line,e.col,e.metadata,e.provenance,ifnull(s.file_path,''),ifnull(t.file_path,'') FROM edges e LEFT JOIN nodes s ON s.id=e.source LEFT JOIN nodes t ON t.id=e.target WHERE e.source IN(SELECT id FROM nodes WHERE file_path=?1 AND kind IN('route','component')) OR e.target IN(SELECT id FROM nodes WHERE file_path=?1 AND kind IN('route','component')) LIMIT 20001",
    ))?;
    let rows = sql(statement.query_map([path], |r| {
        Ok(ProviderEdge {
            source: r.get(0)?,
            target: r.get(1)?,
            kind: r.get(2)?,
            line: r.get(3)?,
            col: r.get(4)?,
            metadata: r.get(5)?,
            provenance: r.get(6)?,
            source_path: r.get(7)?,
            target_path: r.get(8)?,
        })
    }))?;
    let mut result = Vec::new();
    for row in rows {
        let row = sql(row)?;
        if result.len() >= MAX_ROWS {
            return Err(LIMIT);
        }
        result.push(row);
    }
    Ok(result)
}
#[derive(Clone)]
pub(super) struct Incoming {
    pub(super) source: String,
    pub(super) target: String,
    pub(super) kind: String,
    pub(super) line: Option<i64>,
    pub(super) col: Option<i64>,
    pub(super) metadata: Option<String>,
    pub(super) provenance: Option<String>,
    pub(super) path: String,
    pub(super) language: String,
}
pub(super) fn incoming(db: &Connection, path: &str) -> Result<Vec<Incoming>, Failure> {
    let mut statement = sql(db.prepare("SELECT e.source,e.target,e.kind,e.line,e.col,e.metadata,e.provenance,s.file_path,s.language FROM edges e JOIN nodes t ON t.id=e.target JOIN nodes s ON s.id=e.source WHERE t.file_path=?1 AND s.file_path!=?1 AND e.kind!='contains' LIMIT 20001"))?;
    let rows = sql(statement.query_map([path], |r| {
        Ok(Incoming {
            source: r.get(0)?,
            target: r.get(1)?,
            kind: r.get(2)?,
            line: r.get(3)?,
            col: r.get(4)?,
            metadata: r.get(5)?,
            provenance: r.get(6)?,
            path: r.get(7)?,
            language: r.get(8)?,
        })
    }))?;
    let mut result = Vec::new();
    let mut reserved = 0usize;
    for row in rows {
        let row = sql(row)?;
        reserved += row.source.len()
            + row.target.len()
            + row.kind.len()
            + row.path.len()
            + row.language.len()
            + row.metadata.as_ref().map_or(0, String::len)
            + row.provenance.as_ref().map_or(0, String::len);
        if result.len() >= MAX_ROWS || reserved > 16 * 1024 * 1024 {
            return Err(LIMIT);
        }
        result.push(row);
    }
    Ok(result)
}
#[allow(clippy::too_many_arguments)] // Mirrors the provider's unresolved_refs row.
pub(super) fn insert_ref(
    db: &Connection,
    source: &str,
    name: &str,
    kind: &str,
    line: i64,
    col: i64,
    candidates: Option<String>,
    origin: (&str, &str),
) -> Result<i64, Failure> {
    sql(sql(db.prepare_cached("INSERT INTO unresolved_refs(from_node_id,reference_name,reference_kind,line,col,candidates,file_path,language,status,name_tail) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'pending','')"))?.execute(params![source,name,kind,line,col,candidates,origin.0,origin.1]))?;
    Ok(db.last_insert_rowid())
}

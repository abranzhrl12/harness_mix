//! Incremental Razor dependency rechecks, bounded to changed C# names.
use super::{INDEX, LIMIT, MAX_ROWS, array, insert_ref, sql};
use harness_domain::{Failure, execution::OperationContext};
use rusqlite::{Connection, params};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Default)]
pub(super) struct Changes {
    methods: BTreeSet<String>,
    types: BTreeSet<String>,
    controllers: BTreeSet<String>,
}

impl Changes {
    pub(super) fn capture(
        db: &Connection,
        path: &str,
        extracted: Option<&Value>,
    ) -> Result<Self, Failure> {
        let mut changes = Self::default();
        let mut statement = sql(db.prepare_cached(
            "SELECT kind,name,qualified_name FROM nodes WHERE file_path=?1 AND language='csharp' LIMIT 20001",
        ))?;
        let rows = sql(statement.query_map([path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        }))?;
        for (count, row) in rows.enumerate() {
            if count >= MAX_ROWS {
                return Err(LIMIT);
            }
            let (kind, name, qualified) = sql(row)?;
            changes.add(&kind, &name, &qualified);
        }
        if let Some(extracted) = extracted {
            let nodes = array(extracted, "nodes")?;
            if nodes.len() > MAX_ROWS {
                return Err(LIMIT);
            }
            for node in nodes {
                if node["language"] == "csharp" {
                    changes.add(
                        node["kind"].as_str().ok_or(INDEX)?,
                        node["name"].as_str().ok_or(INDEX)?,
                        node["qualifiedName"].as_str().ok_or(INDEX)?,
                    );
                }
            }
        }
        Ok(changes)
    }

    fn add(&mut self, kind: &str, name: &str, qualified: &str) {
        if kind == "method" {
            self.methods.insert(name.into());
        } else if matches!(kind, "class" | "record" | "struct" | "interface") {
            self.types.insert(qualified.replace("::", "."));
            self.types.insert(name.into());
            if let Some(controller) = name.strip_suffix("Controller") {
                self.controllers.insert(controller.to_ascii_lowercase());
            }
        }
    }

    pub(super) fn recheck(
        &self,
        db: &Connection,
        context: &OperationContext,
    ) -> Result<BTreeSet<i64>, Failure> {
        if self.methods.is_empty() && self.types.is_empty() {
            return Ok(BTreeSet::new());
        }
        // Old pending refs acquire the existing indexed failed-tail key once.
        let mut statement = sql(db.prepare_cached(
            "SELECT id,reference_name,reference_kind FROM unresolved_refs WHERE language='razor' AND name_tail='' AND reference_kind IN('calls','mvc_route') LIMIT 20001",
        ))?;
        let pending = sql(sql(statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        }))?
        .collect::<Result<Vec<_>, _>>())?;
        if pending.len() > MAX_ROWS {
            return Err(LIMIT);
        }
        for (id, name, kind) in pending {
            context.check()?;
            mark_failed(db, id, &name, &kind)?;
        }
        let types = serde_json::to_string(&self.types).map_err(|_| INDEX)?;
        let controllers = serde_json::to_string(&self.controllers).map_err(|_| INDEX)?;
        // Once per changed file, never per reference. Match type/controller
        // dependencies, not common method names such as Index: changing one
        // controller must not invalidate all views in a large application.
        let mut statement = sql(db.prepare_cached(
            "SELECT e.source,e.target,e.kind,e.line,e.col,e.metadata,s.file_path,e.id \
             FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target \
             WHERE s.language='razor' AND json_valid(e.metadata) \
             AND json_extract(e.metadata,'$.resolvedBy') IN \
               ('razor-typed-instance-method-subset-v1','razor-absolute-static-method-subset-v1','razor-mvc-literal-route-subset-v1') \
             AND (json_extract(e.metadata,'$.receiverType') IN (SELECT value FROM json_each(?1)) \
               OR (json_extract(e.metadata,'$.resolvedBy')='razor-absolute-static-method-subset-v1' AND \
                 substr(replace(t.qualified_name,'::','.'),1,length(replace(t.qualified_name,'::','.'))-length(t.name)-1) \
                   IN (SELECT value FROM json_each(?1))) \
               OR (json_extract(e.metadata,'$.refKind')='mvc_route' AND \
                 CASE WHEN json_valid(json_extract(e.metadata,'$.refName')) THEN \
                   lower(json_extract(json_extract(e.metadata,'$.refName'),'$.controller')) END \
                   IN (SELECT value FROM json_each(?2)))) LIMIT 20001",
        ))?;
        let rows = sql(statement.query_map(params![types, controllers], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
            ))
        }))?;
        let mut edges = Vec::new();
        let mut reserved = 0usize;
        for row in rows {
            context.check()?;
            let row = sql(row)?;
            reserved += row.0.len() + row.1.len() + row.2.len() + row.5.len() + row.6.len() + 128;
            if edges.len() >= MAX_ROWS || reserved > 16 * 1024 * 1024 {
                return Err(LIMIT);
            }
            edges.push(row);
        }
        let mut refs = BTreeSet::new();
        for (from, _target, _edge_kind, line, col, metadata, path, edge_id) in edges {
            context.check()?;
            let metadata: Value = serde_json::from_str(&metadata).map_err(|_| INDEX)?;
            let name = metadata["refName"].as_str().ok_or(INDEX)?;
            let kind = metadata["refKind"].as_str().ok_or(INDEX)?;
            sql(db.execute("DELETE FROM edges WHERE id=?1", [edge_id]))?;
            refs.insert(insert_ref(
                db,
                &from,
                name,
                kind,
                line.unwrap_or(0),
                col.unwrap_or(0),
                None,
                (&path, "razor"),
            )?);
        }
        let tails = self
            .methods
            .iter()
            .cloned()
            .chain(self.controllers.iter().map(|c| format!("mvc:{c}")))
            .collect::<Vec<_>>();
        let tails = serde_json::to_string(&tails).map_err(|_| INDEX)?;
        let mut statement = sql(db.prepare_cached(
            "SELECT id FROM unresolved_refs WHERE status='failed' AND name_tail IN (SELECT value FROM json_each(?1)) AND language='razor' LIMIT 20001",
        ))?;
        for row in sql(statement.query_map([tails], |r| r.get::<_, i64>(0)))? {
            refs.insert(sql(row)?);
            if refs.len() > MAX_ROWS {
                return Err(LIMIT);
            }
        }
        Ok(refs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_methods_retry_exact_tails_and_preserve_receiver_names() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(include_str!("../../vendor/codegraph-native/schema.sql"))
            .unwrap();
        db.execute_batch("INSERT INTO nodes(id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,updated_at) VALUES \
          ('view','file','Index.cshtml','Index.cshtml','Views/A/Index.cshtml','razor',1,10,0,0,0), \
          ('method','method','Execute','Demo::Controller::Execute','Controller.cs','csharp',1,1,0,0,0), \
          ('type','class','Controller','Demo::Controller','Controller.cs','csharp',1,1,0,0,0), \
          ('other','method','Execute','Other::Controller::Execute','Other.cs','csharp',1,1,0,0,0);").unwrap();
        let metadata = serde_json::json!({"refName":"receiver.Execute","refKind":"calls",
            "resolvedBy":"razor-typed-instance-method-subset-v1","receiverType":"Demo.Controller"})
        .to_string();
        db.execute("INSERT INTO edges(source,target,kind,line,col,metadata) VALUES('view','method','calls',4,2,?1)", [metadata]).unwrap();
        let other = serde_json::json!({"refName":"unrelated.Execute","refKind":"calls",
            "resolvedBy":"razor-typed-instance-method-subset-v1","receiverType":"Other.Controller"})
        .to_string();
        db.execute("INSERT INTO edges(source,target,kind,line,col,metadata) VALUES('view','other','calls',8,2,?1)", [other]).unwrap();
        let pending = insert_ref(
            &db,
            "view",
            "other.Execute",
            "calls",
            5,
            0,
            None,
            ("Views/A/Index.cshtml", "razor"),
        )
        .unwrap();
        let unrelated = insert_ref(
            &db,
            "view",
            "other.ExecuteMore",
            "calls",
            6,
            0,
            None,
            ("Views/A/Index.cshtml", "razor"),
        )
        .unwrap();
        let changes = Changes::capture(&db, "Controller.cs", None).unwrap();
        let refs = changes.recheck(&db, &OperationContext::default()).unwrap();
        assert!(refs.contains(&pending));
        assert!(!refs.contains(&unrelated));
        assert_eq!(refs.len(), 2);
        assert_eq!(
            db.query_row("SELECT count(*) FROM edges", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        let name: String = db
            .query_row(
                "SELECT reference_name FROM unresolved_refs WHERE line=4",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(name, "receiver.Execute");
        // Invalidating again cannot manufacture duplicate edges or refs.
        assert_eq!(
            changes.recheck(&db, &OperationContext::default()).unwrap(),
            refs
        );
    }
}

pub(super) fn mark_failed(db: &Connection, id: i64, name: &str, kind: &str) -> Result<(), Failure> {
    let tail = if kind == "mvc_route" {
        let route: Value = serde_json::from_str(name).map_err(|_| INDEX)?;
        format!(
            "mvc:{}",
            route["controller"]
                .as_str()
                .ok_or(INDEX)?
                .to_ascii_lowercase()
        )
    } else {
        name.rsplit(['.', ':']).next().unwrap_or(name).to_owned()
    };
    sql(db.execute(
        "UPDATE unresolved_refs SET status='failed',name_tail=?1 WHERE id=?2",
        params![tail, id],
    ))?;
    Ok(())
}

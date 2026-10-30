//! SQLite writes for a prepared extraction, kept inside the caller's transaction.
use super::super::{INDEX, LIMIT, MAX_ROWS, batches, hash, sql};
use super::model::ExtractedGraph;
use harness_domain::{Failure, execution::OperationContext};
use rusqlite::{Transaction, params};
use std::collections::BTreeSet;

pub(super) struct FilePublication<'a> {
    pub(super) graph: Option<&'a ExtractedGraph<'a>>,
    pub(super) path: &'a str,
    pub(super) language: &'a str,
    pub(super) bytes: Option<&'a [u8]>,
    pub(super) generated: i64,
    pub(super) now: i64,
    pub(super) provider_nodes: usize,
}

pub(super) struct GraphWriteOutcome {
    pub(super) node_count: usize,
    pub(super) refs: BTreeSet<i64>,
}

pub(super) fn replace_file_graph(
    db: &Transaction<'_>,
    publication: FilePublication<'_>,
    context: &OperationContext,
) -> Result<GraphWriteOutcome, Failure> {
    let FilePublication {
        graph,
        path,
        language,
        bytes,
        generated,
        now,
        provider_nodes,
    } = publication;
    let mut refs = BTreeSet::new();
    let mut node_count = 0;
    if let Some(graph) = graph {
        if graph.nodes.len() > MAX_ROWS {
            return Err(LIMIT);
        }
        batches::insert(
            db,
            "INSERT INTO nodes(id,kind,name,qualified_name,file_path,language,start_line,end_line,start_column,end_column,docstring,signature,visibility,is_exported,is_async,is_static,is_abstract,decorators,type_parameters,return_type,updated_at) VALUES ",
            21,
            &graph.nodes,
            context,
            |node| {
                batches::values(params![
                    node.id,
                    node.kind,
                    node.name,
                    node.qualified_name,
                    path,
                    language,
                    node.start_line,
                    node.end_line,
                    node.start_column,
                    node.end_column,
                    node.docstring,
                    node.signature,
                    node.visibility,
                    node.is_exported,
                    node.is_async,
                    node.is_static,
                    node.is_abstract,
                    node.decorators.map(serde_json::Value::to_string),
                    node.type_parameters.map(serde_json::Value::to_string),
                    node.return_type,
                    now
                ])
            },
        )?;
        node_count = graph.nodes.len() + provider_nodes;
        batches::insert(
            db,
            "INSERT OR IGNORE INTO edges(source,target,kind,line,col,metadata,provenance) VALUES ",
            7,
            &graph.edges,
            context,
            |edge| {
                batches::values(params![
                    edge.source,
                    edge.target,
                    edge.kind,
                    edge.line,
                    edge.column,
                    edge.metadata.map(serde_json::Value::to_string),
                    edge.provenance
                ])
            },
        )?;
        refs.extend(batches::insert_ids(
            db,
            "INSERT INTO unresolved_refs(from_node_id,reference_name,reference_kind,line,col,candidates,file_path,language) VALUES ",
            8,
            &graph.unresolved_references,
            context,
            |reference| {
                batches::values(params![
                    reference.from_node_id,
                    reference.reference_name,
                    reference.reference_kind,
                    reference.line,
                    reference.column,
                    reference.candidates.map(serde_json::Value::to_string),
                    path,
                    language,
                ])
            },
        )?);
        if refs.len() > MAX_ROWS {
            return Err(LIMIT);
        }
        let bytes = bytes.ok_or(INDEX)?;
        // Persist partial coverage in the provider's errors column so later
        // planning retries extraction and never treats this graph as complete.
        let errors = if graph.coverage.is_some_and(|coverage| coverage["status"] == "partial" || coverage["complete"] == false) {
            serde_json::json!([{"code":"SYNC_PARTIAL","coverage":graph.coverage}]).to_string()
        } else {
            "[]".to_owned()
        };
        // modified_at=0 disables CodeGraph's size/mtime fast-path on the next
        // provider sync; don't invent a timestamp from extraction time.
        sql(db.execute("INSERT INTO files(path,content_hash,language,size,modified_at,indexed_at,node_count,errors,generated) VALUES(?1,?2,?3,?4,0,?5,?6,?7,?8) ON CONFLICT(path) DO UPDATE SET content_hash=excluded.content_hash,language=excluded.language,size=excluded.size,modified_at=0,indexed_at=excluded.indexed_at,node_count=excluded.node_count,errors=excluded.errors,generated=excluded.generated",params![path,hash(bytes),language,bytes.len() as i64,now,node_count as i64,errors,generated]))?;
    } else {
        sql(db.execute("DELETE FROM files WHERE path=?1", [path]))?;
    }
    Ok(GraphWriteOutcome { node_count, refs })
}

//! Bounded SCSS graph extraction.
//!
//! The graph mirrors the stylesheet structure: every rule, variable, mixin,
//! function, include and module directive becomes a node, and nesting becomes
//! a `contains` edge. Selector resolution and value semantics are not inferred.
use super::{CONTRACT, framework};
use crate::adapters::parsers::scss::symbols as scss_symbols;
use harness_domain::{Failure, execution::OperationContext, hash};
use serde_json::{Value, json};
use std::ops::Range;

const LIMIT: Failure = (
    "EXTRACTION_LIMIT",
    "SCSS graph exceeds native extraction budget",
);
const MAX_ROWS: usize = 20_000;

pub(super) fn extract(
    path: &str,
    source: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    if source.len() > 1024 * 1024 {
        return Err(LIMIT);
    }
    let digest = hash(source.as_bytes());
    let symbols = scss_symbols::collect(source, &digest, context)?;
    let positions = framework::Positions::new(source);
    let file = framework::file_node(path, source, "scss");
    let file_id = file["id"].as_str().ok_or(CONTRACT)?.to_owned();
    let mut nodes = vec![file];
    let mut edges = Vec::<Value>::new();
    // Nesting is recovered from the byte ranges: the innermost open range owns
    // the statements that follow it.
    let mut open: Vec<(Range<usize>, String)> = Vec::new();
    for symbol in &symbols {
        context.check()?;
        if nodes.len() >= MAX_ROWS || edges.len() >= MAX_ROWS {
            return Err(LIMIT);
        }
        while open
            .last()
            .is_some_and(|(range, _)| range.end <= symbol.bytes.start)
        {
            open.pop();
        }
        let id = framework::id("scss", path, symbol.kind, symbol.bytes.start);
        let (start_line, start_column) = positions.position(symbol.bytes.start)?;
        let (end_line, end_column) = positions.position(symbol.bytes.end)?;
        nodes.push(json!({
            "id":id,
            "kind":symbol.kind,
            "name":symbol.name,
            "qualifiedName":format!("{path}#scss:{}:{}", symbol.kind, symbol.name),
            "filePath":path,
            "language":"scss",
            "startLine":start_line,
            "endLine":end_line,
            "startColumn":start_column,
            "endColumn":end_column,
            "signature":symbol.signature,
            "framework":"scss"
        }));
        let parent = open
            .last()
            .map(|(_, parent)| parent.clone())
            .unwrap_or_else(|| file_id.clone());
        edges.push(json!({"source":parent,"target":id,"kind":"contains","provenance":"heuristic"}));
        open.push((symbol.bytes.clone(), id));
    }
    Ok(json!({
        "nodes":nodes,
        "edges":edges,
        "unresolvedReferences":[],
        "errors":[],
        "coverage":{
            "parser":"scss-ast-v1",
            "limitations":[
                "Selector resolution, value semantics and module imports are not evaluated",
                "Mixin, function and include relationships are structural, not resolved"
            ],
            "notices":[]
        }
    }))
}

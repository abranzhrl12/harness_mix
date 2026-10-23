//! Typed publication outcome, converted to the stable public JSON at the edge.
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};

fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str, Failure> {
    value[key]
        .as_str()
        .filter(|value| !value.is_empty() && value.len() <= 65_536)
        .ok_or(super::super::INDEX)
}

#[derive(Clone, Copy)]
pub(super) struct NodeRow<'a> {
    pub(super) id: &'a str,
    pub(super) kind: &'a str,
    pub(super) name: &'a str,
    pub(super) qualified_name: &'a str,
    pub(super) start_line: Option<i64>,
    pub(super) end_line: Option<i64>,
    pub(super) start_column: Option<i64>,
    pub(super) end_column: Option<i64>,
    pub(super) docstring: Option<&'a str>,
    pub(super) signature: Option<&'a str>,
    pub(super) visibility: Option<&'a str>,
    pub(super) is_exported: bool,
    pub(super) is_async: bool,
    pub(super) is_static: bool,
    pub(super) is_abstract: bool,
    pub(super) decorators: Option<&'a Value>,
    pub(super) type_parameters: Option<&'a Value>,
    pub(super) return_type: Option<&'a str>,
}

#[derive(Clone, Copy)]
pub(super) struct EdgeRow<'a> {
    pub(super) source: &'a str,
    pub(super) target: &'a str,
    pub(super) kind: &'a str,
    pub(super) line: Option<i64>,
    pub(super) column: Option<i64>,
    pub(super) metadata: Option<&'a Value>,
    pub(super) provenance: Option<&'a str>,
}

#[derive(Clone, Copy)]
pub(super) struct UnresolvedReferenceRow<'a> {
    pub(super) from_node_id: &'a str,
    pub(super) reference_name: &'a str,
    pub(super) reference_kind: &'a str,
    pub(super) line: i64,
    pub(super) column: i64,
    pub(super) candidates: Option<&'a Value>,
}

pub(super) struct ExtractedGraph<'a> {
    pub(super) nodes: Vec<NodeRow<'a>>,
    pub(super) duplicate_nodes: usize,
    pub(super) edges: Vec<EdgeRow<'a>>,
    pub(super) unresolved_references: Vec<UnresolvedReferenceRow<'a>>,
    pub(super) coverage: Option<&'a Value>,
}

impl<'a> ExtractedGraph<'a> {
    /// Prepare borrowed row views so SQL storage consumes validated fields
    /// without copying the extraction payload or interpreting JSON in writes.
    pub(super) fn prepare(value: &'a Value, context: &OperationContext) -> Result<Self, Failure> {
        let mut nodes = Vec::new();
        let node_values = super::super::array(value, "nodes")?;
        if node_values.len() > super::super::MAX_ROWS {
            return Err(super::super::LIMIT);
        }
        for node in node_values {
            context.check()?;
            nodes.push(NodeRow {
                id: required_str(node, "id")?,
                kind: required_str(node, "kind")?,
                name: required_str(node, "name")?,
                qualified_name: required_str(node, "qualifiedName")?,
                start_line: node["startLine"].as_i64(),
                end_line: node["endLine"].as_i64(),
                start_column: node["startColumn"].as_i64(),
                end_column: node["endColumn"].as_i64(),
                docstring: super::super::opt_string(node, "docstring"),
                signature: super::super::opt_string(node, "signature"),
                visibility: super::super::opt_string(node, "visibility"),
                is_exported: node["isExported"].as_bool().unwrap_or(false),
                is_async: node["isAsync"].as_bool().unwrap_or(false),
                is_static: node["isStatic"].as_bool().unwrap_or(false),
                is_abstract: node["isAbstract"].as_bool().unwrap_or(false),
                decorators: node.get("decorators"),
                type_parameters: node.get("typeParameters"),
                return_type: super::super::opt_string(node, "returnType"),
            });
        }
        let mut seen_nodes = std::collections::HashMap::new();
        let mut unique_nodes = Vec::with_capacity(nodes.len());
        let mut duplicate_nodes = 0;
        for node in nodes {
            context.check()?;
            if let Some(index) = seen_nodes.get(node.id).copied() {
                let previous: &mut NodeRow<'a> = &mut unique_nodes[index];
                if previous.kind != node.kind
                    || previous.name != node.name
                    || previous.qualified_name != node.qualified_name
                    || matches!(previous.kind, "method" | "function")
                        && previous.signature != node.signature
                {
                    return Err(super::super::INDEX);
                }
                let incoming_start = node.start_line.map(|line| {
                    (line, node.start_column.unwrap_or(i64::MAX))
                });
                let previous_start = previous.start_line.map(|line| {
                    (line, previous.start_column.unwrap_or(i64::MAX))
                });
                if incoming_start.is_some()
                    && previous_start.is_none_or(|prior| incoming_start.unwrap() < prior)
                {
                    previous.start_line = incoming_start.map(|(line, _)| line);
                    previous.start_column = node.start_column;
                }
                let incoming_end = node
                    .end_line
                    .map(|line| (line, node.end_column.unwrap_or(i64::MIN)));
                let previous_end = previous
                    .end_line
                    .map(|line| (line, previous.end_column.unwrap_or(i64::MIN)));
                if incoming_end.is_some()
                    && previous_end.is_none_or(|prior| incoming_end.unwrap() > prior)
                {
                    previous.end_line = incoming_end.map(|(line, _)| line);
                    previous.end_column = node.end_column;
                }
                duplicate_nodes += 1;
            } else {
                seen_nodes.insert(node.id, unique_nodes.len());
                unique_nodes.push(node);
            }
        }
        let mut edges = Vec::new();
        for edge in super::super::array(value, "edges")? {
            context.check()?;
            edges.push(EdgeRow {
                source: required_str(edge, "source")?,
                target: required_str(edge, "target")?,
                kind: required_str(edge, "kind")?,
                line: edge["line"].as_i64(),
                column: edge["column"].as_i64(),
                metadata: edge.get("metadata"),
                provenance: super::super::opt_string(edge, "provenance"),
            });
        }
        let mut unresolved_references = Vec::new();
        for reference in super::super::array(value, "unresolvedReferences")? {
            context.check()?;
            unresolved_references.push(UnresolvedReferenceRow {
                from_node_id: required_str(reference, "fromNodeId")?,
                reference_name: required_str(reference, "referenceName")?,
                reference_kind: required_str(reference, "referenceKind")?,
                line: reference["line"].as_i64().ok_or(super::super::INDEX)?,
                column: reference["column"].as_i64().ok_or(super::super::INDEX)?,
                candidates: reference.get("candidates"),
            });
        }
        Ok(Self {
            nodes: unique_nodes,
            duplicate_nodes,
            edges,
            unresolved_references,
            coverage: value.get("coverage"),
        })
    }
}

pub(super) struct PublicationReport {
    pub(super) path: String,
    pub(super) changed: bool,
    pub(super) extraction_performed: bool,
    pub(super) deleted: bool,
    pub(super) nodes_written: usize,
    pub(super) incoming_edges_retained: usize,
    pub(super) incoming_references_recovered: usize,
    pub(super) provider_nodes_preserved: usize,
    pub(super) provider_edges_retained: usize,
    pub(super) provider_edges_dropped: usize,
    pub(super) import_references_rechecked: usize,
    pub(super) framework_references_rechecked: usize,
    pub(super) razor_references_rechecked: usize,
    pub(super) references_attempted: usize,
    pub(super) references_resolved: usize,
    pub(super) extraction_coverage: Option<Value>,
}

impl PublicationReport {
    pub(super) fn unchanged(path: &str) -> Value {
        json!({"mode":"sync_file","path":path,"changed":false,"extractionPerformed":false,"committed":false,"synchronized":false,"globalFreshnessCertified":false})
    }

    pub(super) fn into_json(self) -> Value {
        let mut report = json!({
            "mode":"sync_file",
            "path":self.path,
            "changed":self.changed,
            "extractionPerformed":self.extraction_performed,
            "deleted":self.deleted,
            "nodesWritten":self.nodes_written,
            "incomingEdgesRetained":self.incoming_edges_retained,
            "incomingReferencesRecovered":self.incoming_references_recovered,
            "providerNodesPreserved":self.provider_nodes_preserved,
            "providerEdgesRetained":self.provider_edges_retained,
            "providerEdgesDropped":self.provider_edges_dropped,
            "importReferencesRechecked":self.import_references_rechecked,
            "frameworkReferencesRechecked":self.framework_references_rechecked,
            "razorReferencesRechecked":self.razor_references_rechecked,
            "referencesAttempted":self.references_attempted,
            "referencesResolved":self.references_resolved,
            "referencesPending":self.references_attempted - self.references_resolved,
            "resolutionPolicy":"exact-name-relative-import-and-framework-subsets",
            "committed":false,
            "synchronized":false,
            "globalFreshnessCertified":false
        });
        if let Some(coverage) = self.extraction_coverage {
            report["extractionCoverage"] = coverage;
        }
        report
    }
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;

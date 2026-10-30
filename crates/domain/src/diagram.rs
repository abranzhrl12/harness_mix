//! Safe, declarative input for D2 diagram rendering.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const MAX_NODES: usize = 24;
pub const MAX_EDGES: usize = 48;
pub const MAX_LABEL_CHARS: usize = 120;
pub const MAX_SOURCE_BYTES: usize = 8 * 1024;
pub const MAX_RENDER_BYTES: u64 = 1_250_000;

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum DiagramFormat { #[default] Png }

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct DiagramNode { pub id: String, pub label: String }

#[derive(Debug, Deserialize, Serialize, Clone)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct DiagramEdge {
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct DiagramArgs {
    pub nodes: Vec<DiagramNode>,
    #[serde(default)] pub edges: Vec<DiagramEdge>,
    #[serde(default)] pub format: DiagramFormat,
}

impl DiagramArgs {
    /// Generate D2 only from quoted labels and canonical identifiers.
    /// Arbitrary source, imports, URLs, icons, and file inputs are not accepted.
    pub fn to_d2(&self) -> Result<String, &'static str> {
        if self.nodes.is_empty() || self.nodes.len() > MAX_NODES || self.edges.len() > MAX_EDGES {
            return Err("Diagram must contain 1-24 nodes and at most 48 edges");
        }
        let mut ids = HashMap::with_capacity(self.nodes.len());
        for (index, node) in self.nodes.iter().enumerate() {
            if node.id.is_empty() || node.id.len() > 64 || !node.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                return Err("Node ids must be unique, non-empty ASCII identifiers (max 64 characters)");
            }
            if ids.insert(node.id.as_str(), index).is_some() { return Err("Node ids must be unique"); }
            validate_label(&node.label)?;
        }
        for edge in &self.edges {
            if !ids.contains_key(edge.from.as_str()) || !ids.contains_key(edge.to.as_str()) {
                return Err("Every edge must refer to an existing node id");
            }
            if let Some(label) = &edge.label { validate_label(label)?; }
        }
        let mut source = String::new();
        for (index, node) in self.nodes.iter().enumerate() {
            source.push_str(&format!("n{index}: {}\n", d2_quote(&node.label)));
        }
        for edge in &self.edges {
            let from = ids[edge.from.as_str()];
            let to = ids[edge.to.as_str()];
            source.push_str(&format!("n{from} -> n{to}"));
            if let Some(label) = &edge.label { source.push_str(&format!(" {{ label: {} }}", d2_quote(label))); }
            source.push('\n');
        }
        if source.len() > MAX_SOURCE_BYTES { return Err("Generated diagram source exceeds 8 KiB"); }
        Ok(source)
    }
}

fn validate_label(label: &str) -> Result<(), &'static str> {
    if label.is_empty() || label.chars().count() > MAX_LABEL_CHARS { return Err("Labels must contain 1-120 characters"); }
    if label.chars().any(|c| c.is_control() || matches!(c, '@' | '$' | '`')) {
        return Err("Labels cannot contain control characters, imports, interpolation, or backticks");
    }
    Ok(())
}

fn d2_quote(value: &str) -> String {
    let mut result = String::with_capacity(value.len() + 2);
    result.push('"');
    for ch in value.chars() {
        match ch { '\\' => result.push_str("\\\\"), '"' => result.push_str("\\\""), _ => result.push(ch) }
    }
    result.push('"');
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quotes_labels_and_replaces_user_ids_with_canonical_ids() {
        let args = DiagramArgs {
            nodes: vec![DiagramNode { id: "api_gateway".into(), label: "API \"gateway\" \\\"quoted\\\"".into() }, DiagramNode { id: "db".into(), label: "Database".into() }],
            edges: vec![DiagramEdge { from: "api_gateway".into(), to: "db".into(), label: Some("stores data".into()) }], format: DiagramFormat::Png,
        };
        let source = args.to_d2().unwrap();
        assert!(source.contains(&format!("n0: {}", d2_quote(&args.nodes[0].label))), "{source}");
        assert!(source.contains(&format!("n0 -> n1 {{ label: {} }}", d2_quote("stores data"))));
        assert!(!source.contains("api_gateway"));
    }
    #[test]
    fn rejects_import_interpolation_backticks_and_control_payloads() {
        for label in ["@secret.d2", "${HOME}", "`command`", "line\nbreak"] {
            let args = DiagramArgs { nodes: vec![DiagramNode { id: "a".into(), label: label.into() }], edges: vec![], format: DiagramFormat::Png };
            assert!(args.to_d2().is_err(), "accepted {label:?}");
        }
    }
    #[test]
    fn enforces_graph_bounds_and_references() {
        let many = (0..=MAX_NODES).map(|n| DiagramNode { id: format!("n{n}"), label: "node".into() }).collect();
        assert!(DiagramArgs { nodes: many, edges: vec![], format: DiagramFormat::Png }.to_d2().is_err());
        let dangling = DiagramArgs { nodes: vec![DiagramNode { id: "a".into(), label: "A".into() }], edges: vec![DiagramEdge { from: "a".into(), to: "missing".into(), label: None }], format: DiagramFormat::Png };
        assert!(dangling.to_d2().is_err());
    }
}






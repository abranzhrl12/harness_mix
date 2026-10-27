//! Conservative local Rust trait implementation relations.
//!
//! This is a syntax-only proof for direct, unconditional declarations in one
//! source file. It deliberately does not resolve imports, aliases, generic
//! traits, external traits, or dispatch at call sites.

use harness_domain::{Failure, execution::OperationContext};
use ra_ap_syntax::{
    AstNode, SyntaxKind, SyntaxNode,
    ast::{self, HasName},
};
use serde_json::{Value, json};

const CONTRACT: Failure = (
    "EXTRACTION_CONTRACT",
    "Rust graph is missing nodes or edges arrays",
);

pub(super) fn enrich(
    path: &str,
    source: &str,
    graph: &mut Value,
    context: &OperationContext,
) -> Result<(), Failure> {
    context.check()?;
    let parsed = ra_ap_syntax::SourceFile::parse(source, ra_ap_syntax::Edition::Edition2024);
    if !parsed.errors().is_empty() {
        return Ok(());
    }
    let root = parsed.tree().syntax().clone();
    // File-level cfg/attributes also condition every declaration in the file.
    if !plain_attrs(&root) {
        return Ok(());
    }
    let root_items: Vec<_> = root.children().collect();

    // Module-scope macro invocations can emit declarations the syntax tree
    // cannot see. In that case local uniqueness is not evidence.
    if root_items
        .iter()
        .any(|item| item.kind() == SyntaxKind::MACRO_CALL)
    {
        return Ok(());
    }

    let traits: Vec<_> = root_items
        .iter()
        .filter_map(|item| ast::Trait::cast(item.clone()))
        .collect();
    if traits.is_empty() {
        return Ok(());
    }

    let nodes: Vec<_> = graph["nodes"]
        .as_array()
        .ok_or(CONTRACT)?
        .iter()
        .filter_map(|node| {
            Some(GraphNode {
                id: node["id"].as_str()?.to_owned(),
                name: node["name"].as_str()?.to_owned(),
                kind: node["kind"].as_str()?.to_owned(),
                path: node["filePath"].as_str()?.to_owned(),
                language: node["language"].as_str()?.to_owned(),
                line: node["startLine"].as_u64()?,
                column: node["startColumn"].as_u64()?,
            })
        })
        .collect();
    let edges = graph["edges"].as_array_mut().ok_or(CONTRACT)?;
    let mut additions: Vec<Value> = Vec::new();

    for item in root_items {
        context.check()?;
        let Some(implementation) = ast::Impl::cast(item) else {
            continue;
        };
        if !plain_attrs(implementation.syntax())
            || !no_generics(implementation.syntax())
            || negative_impl(implementation.syntax())
        {
            continue;
        }
        let Some(trait_path) = implementation.trait_() else {
            continue;
        };
        let trait_name = trait_path.syntax().text().to_string();
        if !simple_identifier(&trait_name) {
            continue;
        }
        let Some(self_ty) = implementation.self_ty() else {
            continue;
        };
        let self_type = self_ty.syntax().text().to_string();
        if !simple_identifier(&self_type) {
            continue;
        }
        let matching_traits: Vec<_> = traits
            .iter()
            .filter(|candidate| {
                candidate
                    .name()
                    .is_some_and(|name| name.text() == trait_name)
            })
            .collect();
        if matching_traits.len() != 1 {
            continue;
        }
        let trait_item = matching_traits[0];
        if !plain_attrs(trait_item.syntax()) || !no_generics(trait_item.syntax()) {
            continue;
        }
        let Some(trait_list) = assoc_item_list(trait_item.syntax()) else {
            continue;
        };
        let Some(impl_list) = assoc_item_list(implementation.syntax()) else {
            continue;
        };
        let trait_methods = method_nodes(&trait_list, false);
        let impl_methods = method_nodes(&impl_list, true);

        for trait_method in &trait_methods {
            context.check()?;
            let matching_impls: Vec<_> = impl_methods
                .iter()
                .filter(|method| method.name == trait_method.name)
                .collect();
            if matching_impls.len() != 1 {
                continue;
            }
            let Some(source_id) =
                graph_node_id(&nodes, path, source, &trait_method.node, &trait_method.name)
            else {
                continue;
            };
            let Some(target_id) = graph_node_id(
                &nodes,
                path,
                source,
                &matching_impls[0].node,
                &matching_impls[0].name,
            ) else {
                continue;
            };
            if source_id == target_id {
                continue;
            }
            let already_present = edges.iter().chain(additions.iter()).any(|edge: &Value| {
                edge["kind"] == "implements"
                    && edge["source"].as_str() == Some(&source_id)
                    && edge["target"].as_str() == Some(&target_id)
            });
            if !already_present {
                additions.push(json!({
                    "source": source_id,
                    "target": target_id,
                    "kind": "implements",
                    "provenance": "heuristic",
                    "metadata": {
                        "resolvedBy": "rust-local-trait-impl-v1",
                        "relationSemantics": "implementation-not-invocation",
                        "trait": trait_name,
                        "selfType": self_type,
                        "method": trait_method.name,
                        "scope": "same-file-top-level",
                        "cfg": "unconditional"
                    }
                }));
            }
        }
    }

    edges.extend(additions);
    Ok(())
}

struct MethodNode {
    name: String,
    node: SyntaxNode,
}

fn method_nodes(list: &SyntaxNode, implementation: bool) -> Vec<MethodNode> {
    list.children()
        .filter_map(ast::Fn::cast)
        .filter_map(|function| {
            if !plain_attrs(function.syntax()) || !no_generics(function.syntax()) {
                return None;
            }
            let has_body = function
                .syntax()
                .children()
                .any(|child| child.kind() == SyntaxKind::BLOCK_EXPR);
            if has_body != implementation {
                return None;
            }
            Some(MethodNode {
                name: function.name()?.text().to_string(),
                node: function.syntax().clone(),
            })
        })
        .collect()
}

fn assoc_item_list(node: &SyntaxNode) -> Option<SyntaxNode> {
    node.children()
        .find(|child| child.kind() == SyntaxKind::ASSOC_ITEM_LIST)
}

struct GraphNode {
    id: String,
    name: String,
    kind: String,
    path: String,
    language: String,
    line: u64,
    column: u64,
}

fn graph_node_id(
    nodes: &[GraphNode],
    path: &str,
    source: &str,
    function: &SyntaxNode,
    name: &str,
) -> Option<String> {
    let start = function.text_range().start();
    let offset = usize::from(start);
    let prefix = source.get(..offset)?;
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix.rsplit('\n').next()?.encode_utf16().count();
    let matching: Vec<_> = nodes
        .iter()
        .filter(|node| {
            node.path == path
                && node.language == "rust"
                && matches!(node.kind.as_str(), "function" | "method")
                && node.name == name
                && node.line == line as u64
                && node.column == column as u64
        })
        .map(|node| node.id.as_str())
        .collect();
    (matching.len() == 1).then(|| matching[0].to_owned())
}

fn plain_attrs(node: &SyntaxNode) -> bool {
    node.children()
        .filter(|child| child.kind() == SyntaxKind::ATTR)
        .all(|child| {
            ast::Attr::cast(child)
                .and_then(|attribute| attribute.simple_name())
                .is_some_and(|name| name == "doc")
        })
}

fn no_generics(node: &SyntaxNode) -> bool {
    !node.descendants().any(|child| {
        matches!(
            child.kind(),
            SyntaxKind::GENERIC_PARAM_LIST | SyntaxKind::WHERE_CLAUSE
        )
    })
}

fn negative_impl(node: &SyntaxNode) -> bool {
    node.children_with_tokens()
        .any(|element| element.as_token().is_some_and(|token| token.text() == "!"))
}

fn simple_identifier(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|character| character == '_' || character.is_alphanumeric())
        && text
            .chars()
            .next()
            .is_some_and(|character| character == '_' || character.is_alphabetic())
}

#[cfg(test)]
#[path = "rust_tests.rs"]
mod tests;

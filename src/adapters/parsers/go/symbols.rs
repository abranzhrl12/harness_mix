use super::syntax::DEFAULT_SOURCE_LIMIT_BYTES;
use crate::adapters::parsers::model::Symbol;
use ast_grep_language::SupportLang;
use harness_domain::{Failure, execution::OperationContext, hash, results::prefix};

pub(in crate::adapters) fn collect(
    source: &str,
    digest: &str,
    context: &OperationContext,
) -> Result<Vec<Symbol>, Failure> {
    context.check()?;
    if source.len() > DEFAULT_SOURCE_LIMIT_BYTES {
        return Err(("PARSER_LIMIT", "Go source exceeds 512 KiB limit"));
    }
    let tree = ast_grep_core::AstGrep::try_new(source, SupportLang::Go)
        .map_err(|_| ("SYNTAX_ERROR", "Go syntax error"))?;
    let root = tree.root();
    if root.get_doc().tree.root_node().has_error() {
        return Err(("SYNTAX_ERROR", "Go syntax error"));
    }
    let mut output = Vec::new();
    let mut visited = 0usize;
    walk(
        &root,
        source,
        digest,
        "",
        context,
        &mut output,
        0,
        &mut visited,
    )?;
    Ok(output)
}

fn walk<D: ast_grep_core::Doc>(
    parent: &ast_grep_core::Node<'_, D>,
    source: &str,
    digest: &str,
    container: &str,
    context: &OperationContext,
    out: &mut Vec<Symbol>,
    depth: usize,
    visited: &mut usize,
) -> Result<(), Failure> {
    if depth > 128 {
        return Err(("PARSER_LIMIT", "Go syntax nesting exceeds 128 levels"));
    }
    for node in parent.children() {
        context.check()?;
        *visited += 1;
        if *visited > 100_000 {
            return Err(("PARSER_LIMIT", "Go syntax node count exceeds 100000"));
        }
        match node.kind().as_ref() {
            "function_declaration" => {
                record(&node, "function", source, digest, container, context, out)?
            }
            "method_declaration" => {
                let receiver = node
                    .field("receiver")
                    .map(receiver_type)
                    .unwrap_or_default();
                let owner = if receiver.is_empty() {
                    container.to_owned()
                } else if container.is_empty() {
                    receiver
                } else {
                    format!("{container}.{receiver}")
                };
                record(&node, "method", source, digest, &owner, context, out)?;
            }
            "type_spec" => {
                let Some(name_node) = node.field("name") else {
                    continue;
                };
                let name = name_node.text().to_string();
                let type_node = node.field("type");
                let alias = type_node.as_ref().is_some_and(|ty| {
                    source
                        .get(name_node.range().end..ty.range().start)
                        .unwrap_or("")
                        .contains('=')
                });
                let kind = if alias {
                    "type_alias"
                } else {
                    match type_node.as_ref().map(|n| n.kind().to_string()).as_deref() {
                        Some("struct_type") => "struct",
                        Some("interface_type") => "interface",
                        _ => "type",
                    }
                };
                record_named(&node, &name, kind, source, digest, container, context, out)?;
                let next = if container.is_empty() {
                    name
                } else {
                    format!("{container}.{name}")
                };
                walk(
                    &node,
                    source,
                    digest,
                    &next,
                    context,
                    out,
                    depth + 1,
                    visited,
                )?;
                continue;
            }
            "type_alias" => {
                if let Some(name) = node.field("name") {
                    record_named(
                        &node,
                        &name.text().to_string(),
                        "type_alias",
                        source,
                        digest,
                        container,
                        context,
                        out,
                    )?;
                }
                continue;
            }
            "import_spec" => {
                let name = node
                    .field("name")
                    .map(|n| n.text().to_string())
                    .unwrap_or_else(|| {
                        node.field("path")
                            .map(|n| n.text().trim_matches(['"', '`']).to_owned())
                            .unwrap_or_default()
                    });
                if !name.is_empty() {
                    record_named(
                        &node, &name, "import", source, digest, container, context, out,
                    )?;
                }
            }
            "field_declaration" => {
                if let Some(names) = node.field("name") {
                    record_named(
                        &node,
                        &names.text().to_string(),
                        "field",
                        source,
                        digest,
                        container,
                        context,
                        out,
                    )?;
                }
            }
            "method_elem" => {
                if let Some(name) = node.field("name") {
                    let parent_type = enclosing_type_name(&node);
                    record_named(
                        &node,
                        &name.text().to_string(),
                        "method",
                        source,
                        digest,
                        &parent_type,
                        context,
                        out,
                    )?;
                }
            }
            _ => {}
        }
        if matches!(
            node.kind().as_ref(),
            "function_declaration"
                | "method_declaration"
                | "import_spec"
                | "field_declaration"
                | "method_elem"
        ) {
            continue;
        }
        walk(
            &node,
            source,
            digest,
            container,
            context,
            out,
            depth + 1,
            visited,
        )?;
    }
    Ok(())
}

fn receiver_type<D: ast_grep_core::Doc>(receiver: ast_grep_core::Node<'_, D>) -> String {
    let mut pending = vec![receiver];
    let mut visited = 0usize;
    while let Some(node) = pending.pop() {
        visited += 1;
        if visited > 64 {
            return String::new();
        }
        if node.kind().as_ref() == "parameter_declaration" {
            if let Some(ty) = node.field("type") {
                return base_type_name(&ty);
            }
        }
        let mut children: Vec<_> = node.children().collect();
        children.reverse();
        pending.extend(children);
    }
    String::new()
}

fn base_type_name<D: ast_grep_core::Doc>(ty: &ast_grep_core::Node<'_, D>) -> String {
    let mut pending = vec![ty.clone()];
    let mut visited = 0usize;
    while let Some(node) = pending.pop() {
        visited += 1;
        if visited > 64 {
            return String::new();
        }
        if matches!(node.kind().as_ref(), "type_identifier" | "identifier") {
            return node.text().to_string();
        }
        let mut children: Vec<_> = node.children().collect();
        children.reverse();
        pending.extend(children);
    }
    String::new()
}

fn enclosing_type_name<D: ast_grep_core::Doc>(node: &ast_grep_core::Node<'_, D>) -> String {
    let mut parent = node.parent();
    while let Some(candidate) = parent {
        if candidate.kind().as_ref() == "type_spec" {
            return candidate
                .field("name")
                .map(|name| name.text().to_string())
                .unwrap_or_default();
        }
        parent = candidate.parent();
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_go_receiver_methods_types_and_imports() {
        let source = "package demo\nimport `fmt`\ntype Box[T any] struct { Value T }\ntype Alias = Box[string]\ntype Runner interface { Run() error }\nfunc (b *Box[T]) Run() error { fmt.Println(b.Value); return nil }\nfunc Café() {}\n";
        let digest = hash(source.as_bytes());
        let symbols = collect(source, &digest, &OperationContext::default()).unwrap();
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Box" && s.kind == "struct")
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Runner" && s.kind == "interface")
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Alias" && s.kind == "type_alias")
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Run" && s.kind == "method" && s.container == "Box")
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Run" && s.kind == "method" && s.container == "Runner")
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Value" && s.kind == "field" && s.container == "Box")
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "Café" && s.kind == "function")
        );
        assert!(symbols.iter().any(|s| s.kind == "import"));
        assert!(
            symbols
                .iter()
                .any(|s| s.kind == "import" && s.name == "fmt")
        );
    }
}

fn record<D: ast_grep_core::Doc>(
    node: &ast_grep_core::Node<'_, D>,
    kind: &'static str,
    source: &str,
    digest: &str,
    container: &str,
    context: &OperationContext,
    out: &mut Vec<Symbol>,
) -> Result<(), Failure> {
    let Some(name) = node.field("name") else {
        return Ok(());
    };
    record_named(
        node,
        &name.text().to_string(),
        kind,
        source,
        digest,
        container,
        context,
        out,
    )
}

fn record_named<D: ast_grep_core::Doc>(
    node: &ast_grep_core::Node<'_, D>,
    name: &str,
    kind: &'static str,
    source: &str,
    digest: &str,
    container: &str,
    context: &OperationContext,
    out: &mut Vec<Symbol>,
) -> Result<(), Failure> {
    context.check()?;
    if out.len() >= 2000 {
        return Err(("PARSER_LIMIT", "Go symbol count exceeds 2000"));
    }
    let range = node.range();
    let (start, end) = (range.start, range.end);
    let text = source
        .get(start..end)
        .ok_or(("RANGE", "Parser offsets incompatible with source"))?;
    let signature_end = if matches!(kind, "function" | "method") {
        node.field("body").map(|n| n.range().start).unwrap_or(end)
    } else {
        end
    };
    let signature = source
        .get(start..signature_end)
        .unwrap_or(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let line = source[..start].bytes().filter(|b| *b == b'\n').count() + 1;
    let end_line = source[..end].bytes().filter(|b| *b == b'\n').count()
        + usize::from(!source[..end].ends_with('\n'));
    let from = source[..start].encode_utf16().count();
    let id = hash(format!("{digest}:{from}:{}", from + text.encode_utf16().count()).as_bytes())
        [..24]
        .to_owned();
    out.push(Symbol {
        name: name.to_owned(),
        container: container.to_owned(),
        kind,
        start: line,
        end: end_line,
        signature: prefix(&signature, 500).to_owned(),
        attributes: Vec::new(),
        doc: String::new(),
        id,
        bytes: start..end,
    });
    Ok(())
}

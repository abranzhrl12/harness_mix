use harness_domain::{Failure, execution::OperationContext, hash};
use serde_json::{Value, json};

pub(super) fn extract(
    path: &str,
    source: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    super::super::parsers::go::syntax::validate_syntax(source, context)?;
    let digest = hash(source.as_bytes());
    let symbols = super::super::parsers::go::symbols::collect(source, &digest, context)?;
    let positions = super::framework::Positions::new(source);
    let file = super::framework::file_node(path, source, "go");
    let file_id = file["id"].as_str().ok_or(super::CONTRACT)?.to_owned();
    let mut nodes = vec![file];
    let mut edges = Vec::new();
    let mut ids = Vec::new();
    for symbol in &symbols {
        context.check()?;
        let kind = match symbol.kind {
            "function" => "function",
            "method" => "method",
            "struct" => "struct",
            "interface" => "interface",
            "type" | "type_alias" => "type_alias",
            "field" => "field",
            "import" => "import",
            _ => continue,
        };
        let id = super::framework::id("go", path, kind, symbol.bytes.start);
        let (start_line, start_column) = positions.position(symbol.bytes.start)?;
        let (end_line, end_column) = positions.position(symbol.bytes.end)?;
        let qualified = if symbol.container.is_empty() {
            symbol.name.clone()
        } else {
            format!("{}.{}", symbol.container, symbol.name)
        };
        nodes.push(json!({"id":id,"kind":kind,"name":symbol.name,"qualifiedName":qualified,"filePath":path,"language":"go","startLine":start_line,"endLine":end_line,"startColumn":start_column,"endColumn":end_column,"signature":symbol.signature}));
        ids.push((symbol, id));
    }
    for (symbol, id) in &ids {
        let is_import = symbol.kind == "import";
        let parent = if !is_import && !symbol.container.is_empty() {
            ids.iter()
                .filter(|(candidate, _)| {
                    candidate.bytes.start <= symbol.bytes.start
                        && candidate.bytes.end >= symbol.bytes.end
                        && matches!(
                            candidate.kind,
                            "struct" | "interface" | "type" | "type_alias"
                        )
                })
                .min_by_key(|(candidate, _)| candidate.bytes.end - candidate.bytes.start)
                .map(|(_, id)| id.clone())
                .or_else(|| {
                    let owner = symbol.container.rsplit('.').next()?;
                    let matching: Vec<_> = ids
                        .iter()
                        .filter(|(candidate, _)| {
                            candidate.kind == "struct" && candidate.name == owner
                        })
                        .map(|(_, id)| id.clone())
                        .collect();
                    (matching.len() == 1).then(|| matching[0].clone())
                })
        } else {
            None
        };
        let source_id = parent.unwrap_or_else(|| file_id.clone());
        edges.push(if is_import { json!({"source":source_id,"target":id,"kind":"imports","provenance":"tree-sitter","metadata":{"resolvedBy":"go-import-declaration-v1","scope":"same-file-syntax"}}) } else { json!({"source":source_id,"target":id,"kind":"contains","provenance":"tree-sitter"}) });
    }
    Ok(
        json!({"nodes":nodes,"edges":edges,"unresolvedReferences":[],"errors":[],"coverage":"Go declarations and imports from tree-sitter syntax; containment and direct import declarations only. No type checking, cross-package name resolution, dynamic calls or interface dispatch."}),
    )
}

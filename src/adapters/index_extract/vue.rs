//! Bounded Vue SFC graph extraction. Script bodies use the native JS/TS
//! kernel through a full-source projection so all emitted coordinates remain
//! relative to the original SFC. Template and style relations are structural
//! candidates only; Vue runtime/type semantics are not inferred here.
use super::{CONTRACT, decode, framework};
use crate::adapters::parsers::vue::structure::{self, Block, Document, Element};
use harness_domain::{Failure, execution::OperationContext, hash};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

const LIMIT: Failure = (
    "EXTRACTION_LIMIT",
    "Vue graph exceeds native extraction budget",
);
const MAX_ROWS: usize = 20_000;

pub(super) fn extract(
    path: &str,
    source: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    if source.len() > 512 * 1024 {
        return Err((
            "EXTRACTION_LIMIT",
            "Vue SFC exceeds native extraction size budget",
        ));
    }

    let digest = hash(source.as_bytes());
    let document = structure::parse(source, &digest, context)?;
    let positions = framework::Positions::new(source);
    let file = framework::file_node(path, source, "vue");
    let file_id = file["id"].as_str().ok_or(CONTRACT)?.to_owned();
    let mut nodes = vec![file];
    let mut edges = Vec::<Value>::new();
    let mut references = Vec::<Value>::new();
    let mut notices = document.notices.clone();
    let component_name = component_name(path);
    let component_id = framework::id("vue", path, "component", 0);
    let component_node = structural_node(
        component_id.clone(),
        "component",
        component_name.to_owned(),
        format!("{path}::component"),
        path,
        "vue",
        &positions,
        0..source.len(),
        Some(component_name.to_owned()),
    )?;
    nodes.push(component_node);
    edges.push(contains(&file_id, &component_id));
    let mut block_nodes = Vec::with_capacity(document.blocks.len());
    let mut script_blocks = 0usize;
    let mut style_blocks = 0usize;
    let mut template_blocks = 0usize;
    let external_blocks = document
        .blocks
        .iter()
        .filter(|block| structure::attr(&block.attrs, "src").is_some())
        .count();
    let external_script_blocks = document
        .blocks
        .iter()
        .filter(|block| block.kind == "script" && structure::attr(&block.attrs, "src").is_some())
        .count();
    let custom_blocks = document
        .blocks
        .iter()
        .filter(|block| !matches!(block.kind.as_str(), "script" | "template" | "style"))
        .count();
    let unsupported_script_languages = document
        .blocks
        .iter()
        .filter(|block| {
            block.kind == "script"
                && structure::attr(&block.attrs, "src").is_none()
                && script_language(block).is_none()
        })
        .count();

    for (index, block) in document.blocks.iter().enumerate() {
        context.check()?;
        let qname = format!("{path}#vue:{}:{index}", block.kind);
        let id = framework::id("vue", path, "module", block.range.start);
        let mut node = structural_node(
            id,
            "module",
            block_name(block, index),
            qname,
            path,
            "vue",
            &positions,
            block.range.clone(),
            Some(block_signature(source, block)),
        )?;
        node["framework"] = json!("vue");
        node["blockKind"] = json!(block.kind);
        node["blockIndex"] = json!(index);
        nodes.push(node);
        block_nodes.push(
            nodes
                .last()
                .and_then(|n| n["id"].as_str())
                .ok_or(CONTRACT)?
                .to_owned(),
        );
        edges.push(contains(&component_id, block_nodes.last().ok_or(CONTRACT)?));

        match block.kind.as_str() {
            "script" => {
                script_blocks += 1;
                extract_script(
                    path,
                    source,
                    index,
                    block,
                    &block_nodes[index],
                    context,
                    &mut nodes,
                    &mut edges,
                    &mut references,
                    &mut notices,
                )?;
            }
            "template" => template_blocks += 1,
            "style" => style_blocks += 1,
            _ => {
                notices.insert(format!(
                    "Custom <{}> block is recorded structurally; its content is not parsed or loaded",
                    block.kind
                ));
            }
        }
        if nodes.len() > MAX_ROWS || edges.len() > MAX_ROWS || references.len() > MAX_ROWS {
            return Err(LIMIT);
        }
    }

    let (component_candidate_count, element_count) = add_template_elements(
        path,
        &document,
        &block_nodes,
        &positions,
        &mut nodes,
        &mut edges,
        context,
    )?;
    let style_rule_count = add_style_rules(
        path,
        &document,
        &block_nodes,
        &positions,
        &mut nodes,
        &mut edges,
        context,
    )?;

    if !document.expressions.is_empty() {
        notices.insert(
            "Vue template interpolations and directive expressions are not resolved".into(),
        );
    }
    if component_candidate_count > 0 {
        notices.insert("Vue template component tags are structural candidates and are not linked to script imports".into());
    }
    context.check()?;
    if nodes.len() > MAX_ROWS || edges.len() > MAX_ROWS || references.len() > MAX_ROWS {
        return Err(LIMIT);
    }

    Ok(json!({
        "nodes":nodes,
        "edges":edges,
        "unresolvedReferences":references,
        "errors":[],
        "coverage":{
            "scripts":{"blocks":script_blocks,"nativeKernel":true,"references":"script-only","externalBlocksNotRead":external_script_blocks,"unsupportedLanguagesSkipped":unsupported_script_languages},
            "template":{"blocks":template_blocks,"elements":element_count,"componentTagCandidates":component_candidate_count,"expressionsResolved":false,"componentImportsResolved":false},
            "styles":{"blocks":style_blocks,"rules":style_rule_count,"selectorSemanticsResolved":false},
            "customBlocks":{"count":custom_blocks,"contentParsed":false},
            "externalBlocksNotRead":external_blocks,
            "vueCompilerOrTypeCheckerRun":false,
            "notices":notices.into_iter().collect::<Vec<_>>()
        }
    }))
}

fn block_name(block: &Block, index: usize) -> String {
    match block.kind.as_str() {
        "script" if structure::attr(&block.attrs, "setup").is_some() => {
            format!("script setup#{index}")
        }
        kind => format!("{kind}#{index}"),
    }
}

fn component_name(path: &str) -> &str {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    if file.to_ascii_lowercase().ends_with(".vue") {
        &file[..file.len() - 4]
    } else {
        file
    }
}

fn block_signature(source: &str, block: &Block) -> String {
    source[block.range.start..block.body.start]
        .trim()
        .to_owned()
}

fn structural_node(
    id: String,
    kind: &str,
    name: String,
    qualified_name: String,
    path: &str,
    language: &str,
    positions: &framework::Positions<'_>,
    range: Range<usize>,
    signature: Option<String>,
) -> Result<Value, Failure> {
    let (start_line, start_column) = positions.position(range.start)?;
    let (end_line, end_column) = positions.position(range.end)?;
    let mut node = json!({
        "id":id,"kind":kind,"name":name,"qualifiedName":qualified_name,
        "filePath":path,"language":language,
        "startLine":start_line,"endLine":end_line,
        "startColumn":start_column,"endColumn":end_column
    });
    if let Some(signature) = signature {
        node["signature"] = json!(signature);
    }
    Ok(node)
}

fn contains(source: &str, target: &str) -> Value {
    json!({"source":source,"target":target,"kind":"contains","provenance":"heuristic"})
}

fn script_language(block: &Block) -> Option<&'static str> {
    match structure::attr(&block.attrs, "lang").unwrap_or("js") {
        "js" | "javascript" => Some("javascript"),
        "ts" | "typescript" => Some("typescript"),
        "jsx" => Some("jsx"),
        "tsx" => Some("tsx"),
        _ => None,
    }
}

fn extract_script(
    path: &str,
    source: &str,
    block_index: usize,
    block: &Block,
    block_id: &str,
    context: &OperationContext,
    nodes: &mut Vec<Value>,
    edges: &mut Vec<Value>,
    references: &mut Vec<Value>,
    notices: &mut BTreeSet<String>,
) -> Result<(), Failure> {
    if structure::attr(&block.attrs, "src").is_some() {
        notices.insert("External script block is not read or resolved".into());
        return Ok(());
    }
    let Some(language) = script_language(block) else {
        notices.insert(format!(
            "Unsupported Vue script lang='{}'; block content was not extracted",
            structure::attr(&block.attrs, "lang").unwrap_or("")
        ));
        return Ok(());
    };

    context.check()?;
    let projection = framework::project(source, block.body.clone())?;
    let raw = codegraph_native::extract(path, &projection, language).map_err(|_| {
        (
            "EXTRACTION_DEFERRED",
            "Native Vue script extraction deferred this source; no partial graph was returned",
        )
    })?;
    let extracted = decode::decode(&raw, path, "vue", context)?;
    let raw_nodes = extracted["nodes"].as_array().ok_or(CONTRACT)?;
    let raw_file_id = raw_nodes
        .iter()
        .find(|node| node["kind"] == "file")
        .and_then(|node| node["id"].as_str())
        .ok_or(CONTRACT)?;
    let mut id_map = BTreeMap::<String, String>::new();
    id_map.insert(raw_file_id.to_owned(), block_id.to_owned());
    for node in raw_nodes {
        context.check()?;
        if node["kind"] == "file" {
            continue;
        }
        let old_id = node["id"].as_str().ok_or(CONTRACT)?;
        let new_id = framework::id(
            &format!("vue_script:{block_index}"),
            path,
            old_id,
            block.body.start,
        );
        if id_map.insert(old_id.to_owned(), new_id).is_some() {
            return Err(CONTRACT);
        }
    }
    for node in raw_nodes {
        context.check()?;
        if node["kind"] == "file" {
            continue;
        }
        if nodes.len() >= MAX_ROWS {
            return Err(LIMIT);
        }
        let mut node = node.clone();
        node["id"] = json!(
            id_map
                .get(node["id"].as_str().ok_or(CONTRACT)?)
                .ok_or(CONTRACT)?
        );
        nodes.push(node);
    }
    for edge in extracted["edges"].as_array().ok_or(CONTRACT)? {
        context.check()?;
        let mut edge = edge.clone();
        edge["source"] = json!(
            id_map
                .get(edge["source"].as_str().ok_or(CONTRACT)?)
                .ok_or(CONTRACT)?
        );
        edge["target"] = json!(
            id_map
                .get(edge["target"].as_str().ok_or(CONTRACT)?)
                .ok_or(CONTRACT)?
        );
        if edges.len() >= MAX_ROWS {
            return Err(LIMIT);
        }
        edges.push(edge);
    }
    for reference in extracted["unresolvedReferences"]
        .as_array()
        .ok_or(CONTRACT)?
    {
        context.check()?;
        let mut reference = reference.clone();
        reference["fromNodeId"] = json!(
            id_map
                .get(reference["fromNodeId"].as_str().ok_or(CONTRACT)?)
                .ok_or(CONTRACT)?
        );
        if references.len() >= MAX_ROWS {
            return Err(LIMIT);
        }
        references.push(reference);
    }

    // Extraer llamadas a provide('key', ...) e inject('key') con clave literal
    extract_vue_provide_inject(source, block, block_id, references)?;
    Ok(())
}

fn extract_vue_provide_inject(
    source: &str,
    block: &Block,
    block_id: &str,
    references: &mut Vec<Value>,
) -> Result<(), Failure> {
    let body_src = &source[block.body.clone()];
    let allocator = oxc_allocator::Allocator::default();
    let source_type = oxc_span::SourceType::default();
    let parsed = oxc_parser::Parser::new(&allocator, body_src, source_type).parse();
    if parsed.diagnostics.has_errors() {
        return Ok(());
    }

    let positions = framework::Positions::new(source);
    let offset = block.body.start;

    for stmt in &parsed.program.body {
        scan_stmt_for_provide_inject(stmt, offset, source, block_id, &positions, references)?;
    }
    Ok(())
}

fn scan_stmt_for_provide_inject(
    stmt: &oxc_ast::ast::Statement<'_>,
    offset: usize,
    source: &str,
    block_id: &str,
    positions: &framework::Positions<'_>,
    references: &mut Vec<Value>,
) -> Result<(), Failure> {
    use oxc_ast::ast::*;
    match stmt {
        Statement::ExpressionStatement(expr_stmt) => {
            scan_expr_for_provide_inject(&expr_stmt.expression, offset, source, block_id, positions, references)?;
        }
        Statement::VariableDeclaration(var_decl) => {
            for decl in &var_decl.declarations {
                if let Some(init) = &decl.init {
                    scan_expr_for_provide_inject(init, offset, source, block_id, positions, references)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn scan_expr_for_provide_inject(
    expr: &oxc_ast::ast::Expression<'_>,
    offset: usize,
    _source: &str,
    block_id: &str,
    positions: &framework::Positions<'_>,
    references: &mut Vec<Value>,
) -> Result<(), Failure> {
    use oxc_ast::ast::*;
    use oxc_span::GetSpan;
    if let Expression::CallExpression(call) = expr {
        if let Expression::Identifier(ident) = &call.callee {
            let fn_name = ident.name.as_str();
            if fn_name == "provide" || fn_name == "inject" {
                if let Some(first_arg) = call.arguments.first() {
                    let key = match first_arg {
                        Argument::StringLiteral(s) => Some(s.value.display().to_string()),
                        _ => None,
                    };
                    if let Some(ref_name) = key {
                        let span = first_arg.span();
                        let abs_start = offset + span.start as usize;
                        let (line, column) = positions.position(abs_start)?;
                        let kind = if fn_name == "provide" {
                            "vue_provide"
                        } else {
                            "vue_inject"
                        };
                        references.push(json!({
                            "fromNodeId": block_id,
                            "referenceName": ref_name,
                            "referenceKind": kind,
                            "line": line,
                            "column": column,
                        }));
                    }
                }
            }
        }
    }
    Ok(())
}

fn add_template_elements(
    path: &str,
    document: &Document,
    block_nodes: &[String],
    positions: &framework::Positions<'_>,
    nodes: &mut Vec<Value>,
    edges: &mut Vec<Value>,
    context: &OperationContext,
) -> Result<(usize, usize), Failure> {
    let Some((template_index, _)) = document
        .blocks
        .iter()
        .enumerate()
        .find(|(_, block)| block.kind == "template")
    else {
        return Ok((0, 0));
    };
    let mut component_ids = vec![None::<String>; document.elements.len()];
    let mut count = 0;
    for (index, element) in document.elements.iter().enumerate() {
        context.check()?;
        if !is_component(element) {
            continue;
        }
        let id = framework::id("vue", path, "module", element.range.start);
        let node = structural_node(
            id.clone(),
            "module",
            element.tag.clone(),
            format!("{path}#template:component:{index}:{}", element.tag),
            path,
            "vue",
            positions,
            element.range.clone(),
            Some(element.tag.clone()),
        )?;
        let mut node = node;
        node["componentTagCandidate"] = json!(true);
        nodes.push(node);
        component_ids[index] = Some(id);
        count += 1;
        if nodes.len() > MAX_ROWS {
            return Err(LIMIT);
        }
    }
    for (index, element) in document.elements.iter().enumerate() {
        context.check()?;
        let Some(target) = component_ids[index].as_deref() else {
            continue;
        };
        let mut parent = element.parent;
        let mut source_id = block_nodes[template_index].as_str();
        while let Some(parent_index) = parent {
            if let Some(parent_id) = component_ids.get(parent_index).and_then(Option::as_deref) {
                source_id = parent_id;
                break;
            }
            parent = document.elements.get(parent_index).and_then(|e| e.parent);
        }
        edges.push(contains(source_id, target));
        if edges.len() > MAX_ROWS {
            return Err(LIMIT);
        }
    }
    Ok((count, document.elements.len()))
}

fn is_component(element: &Element) -> bool {
    element.tag.chars().next().is_some_and(char::is_uppercase)
        || element.tag.contains('-')
        || element.tag == "component"
}

fn add_style_rules(
    path: &str,
    document: &Document,
    block_nodes: &[String],
    positions: &framework::Positions<'_>,
    nodes: &mut Vec<Value>,
    edges: &mut Vec<Value>,
    context: &OperationContext,
) -> Result<usize, Failure> {
    let mut rule_ids = vec![None::<String>; document.rules.len()];
    for (index, rule) in document.rules.iter().enumerate() {
        context.check()?;
        let id = framework::id("vue", path, "module", rule.range.start);
        let block_index = rule.block;
        let name = rule.selector.clone();
        let node = structural_node(
            id.clone(),
            "module",
            name.clone(),
            format!("{path}#style:{block_index}:rule:{index}:{name}"),
            path,
            "vue",
            positions,
            rule.range.clone(),
            Some(name),
        )?;
        nodes.push(node);
        rule_ids[index] = Some(id);
        let parent_id = rule
            .parent
            .and_then(|parent| rule_ids.get(parent).and_then(Option::as_deref))
            .or_else(|| block_nodes.get(block_index).map(String::as_str))
            .ok_or(CONTRACT)?;
        edges.push(contains(
            parent_id,
            rule_ids[index].as_deref().ok_or(CONTRACT)?,
        ));
        if nodes.len() > MAX_ROWS || edges.len() > MAX_ROWS {
            return Err(LIMIT);
        }
    }
    Ok(document.rules.len())
}

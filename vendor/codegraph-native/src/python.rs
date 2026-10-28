//! Python extraction walker for CodeGraph native buffers.

use crate::buffers::{
    build_meta, edge_kind_index, node_kind_index, Arena, BoolFlags, EdgeRow, EmitOut, NodeRow,
    RefRow, Tables, FLAG_IS_ASYNC, FLAG_IS_EXPORTED, FLAG_IS_STATIC, NONE, NONE_STR,
};
use crate::docstring::preceding_docstring;
use crate::ids;
use crate::textutil as util;
use tree_sitter::{Node, Parser};

struct Scope {
    row: u32,
    kind: &'static str,
    name: String,
}

#[derive(Default)]
struct Extra {
    docstring: Option<String>,
    signature: Option<String>,
    visibility: Option<u8>,
    is_exported: Option<bool>,
    is_async: Option<bool>,
    is_static: Option<bool>,
    decorators: Vec<String>,
    return_type: Option<String>,
}

pub struct Walker<'t> {
    src: &'t str,
    file_path: &'t str,
    line_starts: Vec<usize>,
    arena: Arena,
    node_id_allocator: ids::NodeIdAllocator,
    tables: Tables,
    stack: Vec<Scope>,
}

pub fn extract(file_path: &str, source: &str) -> Result<EmitOut, String> {
    let grammar = crate::langs::grammar_for("python").ok_or("no python grammar")?;
    let t0 = std::time::Instant::now();
    let mut parser = Parser::new();
    parser
        .set_language(&grammar)
        .map_err(|e| format!("set_language(python) failed: {e}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "parser returned null tree".to_string())?;
    if tree.root_node().has_error() {
        return Err("defer: parse tree contains errors — wasm recovery is canonical".to_string());
    }

    let mut w = Walker {
        src: source,
        file_path,
        line_starts: util::line_starts(source),
        arena: Arena::default(),
        node_id_allocator: ids::NodeIdAllocator::default(),
        tables: Tables::default(),
        stack: Vec::new(),
    };

    let line_count = source.bytes().filter(|b| *b == b'\n').count() as u32 + 1;
    let base_name = file_path.rsplit(['/', '\\']).next().unwrap_or(file_path);
    let mut flags = BoolFlags::default();
    flags.set(FLAG_IS_EXPORTED, false);
    let file_id = w.arena.put(&ids::file_node_id(file_path));
    let name_ref = w.arena.put(base_name);
    let qn_ref = w.arena.put(file_path);
    w.tables.push_node(&NodeRow {
        kind: node_kind_index("file").unwrap(),
        visibility: 0,
        flags,
        start_line: 1,
        end_line: line_count,
        start_column: 0,
        end_column: 0,
        name: name_ref,
        qualified_name: qn_ref,
        id: file_id,
        docstring: NONE_STR,
        signature: NONE_STR,
        decorators: NONE_STR,
        type_parameters: NONE_STR,
        return_type: NONE_STR,
        extra_json: NONE_STR,
    });
    w.stack.push(Scope {
        row: 0,
        kind: "file",
        name: base_name.to_string(),
    });

    w.visit_node(tree.root_node());
    w.stack.pop();

    let duration_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let meta = build_meta(&w.tables, w.arena.len(), NONE_STR, duration_ms);
    Ok(EmitOut {
        meta,
        nodes: w.tables.nodes,
        edges: w.tables.edges,
        refs: w.tables.refs,
        arena: w.arena.into_vec(),
    })
}

impl<'t> Walker<'t> {
    fn text(&self, node: Node) -> &'t str {
        &self.src[node.byte_range()]
    }

    fn line_of(&self, node: Node) -> u32 {
        node.start_position().row as u32 + 1
    }

    fn col_of(&self, node: Node) -> u32 {
        util::col16(
            self.src,
            &self.line_starts,
            node.start_position().row,
            node.start_byte(),
        )
    }

    fn end_col_of(&self, node: Node) -> u32 {
        util::col16(
            self.src,
            &self.line_starts,
            node.end_position().row,
            node.end_byte(),
        )
    }

    fn top_row(&self) -> u32 {
        self.stack.last().map(|s| s.row).unwrap_or(0)
    }

    fn inside_class(&self) -> bool {
        self.stack.last().is_some_and(|s| s.kind == "class")
    }

    fn push_ref_at(&mut self, from_row: u32, name: &str, kind_code: u8, node: Node) {
        if name.is_empty() {
            return;
        }
        let name_ref = self.arena.put(name);
        self.tables.push_ref(&RefRow {
            from_idx: from_row,
            kind: kind_code,
            line: self.line_of(node),
            column: self.col_of(node),
            reference_name: name_ref,
            candidates: NONE_STR,
            from_id_str: NONE_STR,
        });
    }

    fn create_node(
        &mut self,
        kind: &'static str,
        name: &str,
        node: Node<'t>,
        extra: Extra,
    ) -> Option<u32> {
        if name.is_empty() {
            return None;
        }
        let span_node = match node.parent() {
            Some(p) if p.kind() == "decorated_definition" => p,
            _ => node,
        };
        let start_line = self.line_of(span_node);
        let column = self.col_of(span_node);
        let id = self
            .node_id_allocator
            .generate(self.file_path, kind, name, start_line, column);
        let end_line = span_node.end_position().row as u32 + 1;

        let qualified = {
            let mut parts: Vec<&str> = Vec::new();
            for s in &self.stack {
                if s.kind != "file" {
                    parts.push(&s.name);
                }
            }
            let mut qn = parts.join("::");
            if !qn.is_empty() {
                qn.push_str("::");
            }
            qn.push_str(name);
            qn
        };

        let mut flags = BoolFlags::default();
        if let Some(v) = extra.is_exported {
            flags.set(FLAG_IS_EXPORTED, v);
        }
        if let Some(v) = extra.is_async {
            flags.set(FLAG_IS_ASYNC, v);
        }
        if let Some(v) = extra.is_static {
            flags.set(FLAG_IS_STATIC, v);
        }
        let name_ref = self.arena.put(name);
        let qn_ref = self.arena.put(&qualified);
        let id_ref = self.arena.put(&id);
        let doc_ref = self.arena.put_opt(extra.docstring.as_deref());
        let sig_ref = self.arena.put_opt(extra.signature.as_deref());
        let decorators_ref = self.arena.put_list(&extra.decorators);
        let ret_ref = self.arena.put_opt(extra.return_type.as_deref());
        let row = self.tables.push_node(&NodeRow {
            kind: node_kind_index(kind).unwrap(),
            visibility: extra.visibility.unwrap_or(0),
            flags,
            start_line,
            end_line,
            start_column: self.col_of(span_node),
            end_column: self.end_col_of(span_node),
            name: name_ref,
            qualified_name: qn_ref,
            id: id_ref,
            docstring: doc_ref,
            signature: sig_ref,
            decorators: decorators_ref,
            type_parameters: NONE_STR,
            return_type: ret_ref,
            extra_json: NONE_STR,
        });

        let parent_row = self.top_row();
        self.tables.push_edge(&EdgeRow {
            source_idx: parent_row,
            target_idx: row,
            kind: edge_kind_index("contains").unwrap(),
            provenance: 0,
            line: NONE,
            column: NONE,
            metadata_json: NONE_STR,
            source_id_str: NONE_STR,
            target_id_str: NONE_STR,
        });

        Some(row)
    }

    fn extract_name(&self, node: Node) -> String {
        if let Some(name_node) = node.child_by_field_name("name") {
            return self.text(name_node).to_string();
        }
        for i in 0..node.named_child_count() {
            if let Some(c) = node.named_child(i) {
                if c.kind() == "identifier" {
                    return self.text(c).to_string();
                }
            }
        }
        "<anonymous>".to_string()
    }

    fn visibility_of(name: &str) -> u8 {
        if name.starts_with("__") && name.ends_with("__") && name.len() >= 4 {
            1 // public (dunder)
        } else if name.starts_with("__") {
            2 // private
        } else if name.starts_with('_') {
            3 // protected
        } else {
            1 // public
        }
    }

    fn is_async(&self, node: Node) -> bool {
        (0..node.child_count())
            .filter_map(|i| node.child(i))
            .any(|c| c.kind() == "async" || self.text(c) == "async")
    }

    fn signature_of(&self, node: Node) -> Option<String> {
        let params = node.child_by_field_name("parameters")?;
        let mut sig = self.text(params).to_string();
        if let Some(rt) = node.child_by_field_name("return_type") {
            sig.push_str(" -> ");
            sig.push_str(self.text(rt));
        }
        Some(sig)
    }

    fn return_type_of(&self, node: Node) -> Option<String> {
        let rt = node.child_by_field_name("return_type")?;
        let raw = self.text(rt).trim();
        if raw.is_empty() || raw == "None" {
            return None;
        }
        let base = raw.split('[').next().unwrap_or(raw).trim();
        let last = base.rsplit('.').next().unwrap_or(base).trim();
        if last.is_empty() {
            None
        } else {
            Some(last.to_string())
        }
    }

    fn docstring_of(&self, node: Node<'t>) -> Option<String> {
        if let Some(body) = node.child_by_field_name("body") {
            if let Some(first_stmt) = body.named_child(0) {
                let expr = if first_stmt.kind() == "expression_statement" {
                    first_stmt.named_child(0)
                } else if first_stmt.kind() == "string" {
                    Some(first_stmt)
                } else {
                    None
                };
                if let Some(str_node) = expr.filter(|n| n.kind() == "string") {
                    if let Some(cleaned) = clean_python_docstring(self.text(str_node)) {
                        return Some(cleaned);
                    }
                }
            }
        }
        preceding_docstring(node, self.src)
    }

    fn decorators_of(&self, node: Node<'t>) -> (Vec<String>, Vec<(String, Node<'t>)>) {
        let mut list = Vec::new();
        let mut refs = Vec::new();
        let Some(parent) = node.parent() else {
            return (list, refs);
        };
        if parent.kind() != "decorated_definition" {
            return (list, refs);
        }
        for i in 0..parent.named_child_count() {
            let Some(child) = parent.named_child(i) else {
                continue;
            };
            if child.kind() != "decorator" {
                continue;
            }
            let raw = self.text(child).trim().trim_start_matches('@').trim();
            if !raw.is_empty() {
                list.push(raw.to_string());
            }
            if let Some(expr) = child.named_child(0) {
                let target = if expr.kind() == "call" {
                    expr.child_by_field_name("function")
                        .or_else(|| expr.named_child(0))
                } else {
                    Some(expr)
                };
                if let Some(target) = target {
                    let target_text = self.text(target).trim();
                    let name = target_text.rsplit('.').next().unwrap_or(target_text).trim();
                    if !name.is_empty() {
                        refs.push((name.to_string(), child));
                    }
                }
            }
        }
        (list, refs)
    }

    fn visit_node(&mut self, node: Node<'t>) {
        stack_guard!();
        let kind = node.kind();
        let mut skip_children = false;

        match kind {
            "class_definition" => {
                self.extract_class(node);
                skip_children = true;
            }
            "function_definition" => {
                self.extract_function_or_method(node);
                skip_children = true;
            }
            "import_statement" => {
                self.extract_import_statement(node);
                skip_children = true;
            }
            "import_from_statement" => {
                self.extract_import_from_statement(node);
                skip_children = true;
            }
            "call" => {
                self.extract_call(node);
            }
            _ => {}
        }

        if !skip_children {
            for i in 0..node.named_child_count() {
                if let Some(c) = node.named_child(i) {
                    self.visit_node(c);
                }
            }
        }
    }

    fn extract_class(&mut self, node: Node<'t>) {
        stack_guard!();
        let name = self.extract_name(node);
        if name == "<anonymous>" {
            return;
        }
        let (decorators, decorator_refs) = self.decorators_of(node);
        let vis = Self::visibility_of(&name);
        let extra = Extra {
            docstring: self.docstring_of(node),
            visibility: Some(vis),
            is_exported: Some(vis == 1),
            decorators,
            ..Extra::default()
        };
        let Some(row) = self.create_node("class", &name, node, extra) else {
            return;
        };

        self.extract_inheritance(node, row);
        let decorates_kind = edge_kind_index("decorates").unwrap();
        for (dec_name, dec_node) in decorator_refs {
            self.push_ref_at(row, &dec_name, decorates_kind, dec_node);
        }

        self.stack.push(Scope {
            row,
            kind: "class",
            name,
        });
        if let Some(body) = node.child_by_field_name("body") {
            for i in 0..body.named_child_count() {
                if let Some(c) = body.named_child(i) {
                    self.visit_node(c);
                }
            }
        }
        self.stack.pop();
    }

    fn extract_function_or_method(&mut self, node: Node<'t>) {
        stack_guard!();
        let name = self.extract_name(node);
        if name == "<anonymous>" {
            if let Some(body) = node.child_by_field_name("body") {
                self.visit_node(body);
            }
            return;
        }
        let as_method = self.inside_class();
        let kind: &'static str = if as_method { "method" } else { "function" };
        let (decorators, decorator_refs) = self.decorators_of(node);
        let is_static = as_method
            && decorators
                .iter()
                .any(|d| d == "staticmethod" || d == "classmethod");
        let vis = Self::visibility_of(&name);
        let extra = Extra {
            docstring: self.docstring_of(node),
            signature: self.signature_of(node),
            visibility: Some(vis),
            is_exported: if as_method { None } else { Some(vis == 1) },
            is_async: Some(self.is_async(node)),
            is_static: if as_method { Some(is_static) } else { None },
            decorators,
            return_type: self.return_type_of(node),
        };
        let Some(row) = self.create_node(kind, &name, node, extra) else {
            return;
        };

        let decorates_kind = edge_kind_index("decorates").unwrap();
        for (dec_name, dec_node) in decorator_refs {
            self.push_ref_at(row, &dec_name, decorates_kind, dec_node);
        }

        self.stack.push(Scope { row, kind, name });
        if let Some(body) = node.child_by_field_name("body") {
            for i in 0..body.named_child_count() {
                if let Some(c) = body.named_child(i) {
                    self.visit_node(c);
                }
            }
        }
        self.stack.pop();
    }

    fn extract_inheritance(&mut self, node: Node<'t>, class_row: u32) {
        let Some(superclasses) = node.child_by_field_name("superclasses").or_else(|| {
            (0..node.named_child_count())
                .filter_map(|i| node.named_child(i))
                .find(|c| c.kind() == "argument_list")
        }) else {
            return;
        };
        let extends_kind = edge_kind_index("extends").unwrap();
        for i in 0..superclasses.named_child_count() {
            let Some(arg) = superclasses.named_child(i) else {
                continue;
            };
            if arg.kind() == "keyword_argument" {
                continue;
            }
            let base_node = if arg.kind() == "subscript" {
                arg.child_by_field_name("value")
                    .or_else(|| arg.named_child(0))
                    .unwrap_or(arg)
            } else {
                arg
            };
            let text = self.text(base_node).trim();
            let base_name = text.rsplit('.').next().unwrap_or(text).trim();
            if !base_name.is_empty() && base_name != "object" {
                self.push_ref_at(class_row, base_name, extends_kind, arg);
            }
        }
    }

    fn extract_import_statement(&mut self, node: Node<'t>) {
        let signature = self.text(node).trim().to_string();
        let parent = self.top_row();
        let imports_kind = edge_kind_index("imports").unwrap();
        let mut first_module: Option<String> = None;
        let mut modules: Vec<(String, Node<'t>)> = Vec::new();

        for i in 0..node.named_child_count() {
            let Some(child) = node.named_child(i) else {
                continue;
            };
            let target = match child.kind() {
                "dotted_name" => Some(child),
                "aliased_import" => child
                    .child_by_field_name("name")
                    .or_else(|| child.named_child(0)),
                _ => None,
            };
            if let Some(t) = target {
                let name = self.text(t).trim().to_string();
                if !name.is_empty() {
                    if first_module.is_none() {
                        first_module = Some(name.clone());
                    }
                    modules.push((name, t));
                }
            }
        }

        if let Some(module_name) = first_module {
            self.create_node(
                "import",
                &module_name,
                node,
                Extra {
                    signature: Some(signature),
                    ..Extra::default()
                },
            );
        }
        for (mod_name, mod_node) in modules {
            self.push_ref_at(parent, &mod_name, imports_kind, mod_node);
        }
    }

    fn extract_import_from_statement(&mut self, node: Node<'t>) {
        let signature = self.text(node).trim().to_string();
        let parent = self.top_row();
        let imports_kind = edge_kind_index("imports").unwrap();

        let module_node = node.child_by_field_name("module_name").or_else(|| {
            (0..node.named_child_count())
                .filter_map(|i| node.named_child(i))
                .find(|c| matches!(c.kind(), "relative_import" | "dotted_name"))
        });

        let module_name = module_node
            .map(|m| self.text(m).trim().to_string())
            .filter(|s| !s.is_empty());

        if let Some(ref mod_name) = module_name {
            self.create_node(
                "import",
                mod_name,
                node,
                Extra {
                    signature: Some(signature),
                    ..Extra::default()
                },
            );
            self.push_ref_at(parent, mod_name, imports_kind, module_node.unwrap_or(node));
        }

        // Collect imported bindings after `import` token
        let mut after_import_kw = false;
        for i in 0..node.child_count() {
            let Some(child) = node.child(i) else {
                continue;
            };
            if !child.is_named() && self.text(child) == "import" {
                after_import_kw = true;
                continue;
            }
            if !after_import_kw || !child.is_named() {
                continue;
            }
            let target = match child.kind() {
                "dotted_name" | "identifier" => Some(child),
                "aliased_import" => child
                    .child_by_field_name("name")
                    .or_else(|| child.named_child(0)),
                _ => None,
            };
            if let Some(t) = target {
                let binding = self.text(t).trim();
                if !binding.is_empty() && binding != "*" {
                    self.push_ref_at(parent, binding, imports_kind, t);
                }
            }
        }
    }

    fn extract_call(&mut self, node: Node<'t>) {
        if self.stack.is_empty() {
            return;
        }
        let Some(func) = node
            .child_by_field_name("function")
            .or_else(|| node.named_child(0))
        else {
            return;
        };

        let mut callee_name = String::new();
        match func.kind() {
            "identifier" => {
                callee_name = self.text(func).trim().to_string();
            }
            "attribute" => {
                let attr = func
                    .child_by_field_name("attribute")
                    .or_else(|| func.named_child(1));
                let obj = func
                    .child_by_field_name("object")
                    .or_else(|| func.named_child(0));
                if let Some(attr) = attr {
                    let method_name = self.text(attr).trim();
                    if let Some(obj) = obj {
                        if is_literal_receiver(obj.kind()) {
                            return;
                        }
                        if obj.kind() == "identifier" {
                            let recv = self.text(obj).trim();
                            if matches!(recv, "self" | "cls" | "super" | "this") {
                                callee_name = method_name.to_string();
                            } else {
                                callee_name = format!("{recv}.{method_name}");
                            }
                        } else if obj.kind() == "call" {
                            if let Some(inner_fn) = obj.child_by_field_name("function") {
                                if inner_fn.kind() == "identifier"
                                    && self.text(inner_fn).trim() == "super"
                                {
                                    callee_name = method_name.to_string();
                                } else {
                                    let inner: String = self
                                        .text(inner_fn)
                                        .chars()
                                        .filter(|c| !c.is_whitespace())
                                        .collect();
                                    if !inner.is_empty() {
                                        callee_name = format!("{inner}().{method_name}");
                                    } else {
                                        callee_name = method_name.to_string();
                                    }
                                }
                            } else {
                                callee_name = method_name.to_string();
                            }
                        } else {
                            callee_name = method_name.to_string();
                        }
                    } else {
                        callee_name = method_name.to_string();
                    }
                }
            }
            _ => {
                callee_name = self.text(func).trim().to_string();
            }
        }

        if !callee_name.is_empty() {
            if let Some(c) = util::paren_conversion().captures(&callee_name) {
                callee_name = c[1].to_string();
            }
            let from = self.top_row();
            self.push_ref_at(
                from,
                &callee_name,
                edge_kind_index("calls").unwrap(),
                node,
            );
        }
    }
}

fn clean_python_docstring(raw: &str) -> Option<String> {
    let mut s = raw.trim();
    while s.starts_with(|c: char| matches!(c, 'r' | 'R' | 'u' | 'U' | 'f' | 'F' | 'b' | 'B')) {
        s = &s[1..];
    }
    let body = if let Some(inner) = s
        .strip_prefix("\"\"\"")
        .and_then(|rest| rest.strip_suffix("\"\"\""))
    {
        inner
    } else if let Some(inner) = s
        .strip_prefix("'''")
        .and_then(|rest| rest.strip_suffix("'''"))
    {
        inner
    } else if let Some(inner) = s.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')) {
        inner
    } else if let Some(inner) = s
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''))
    {
        inner
    } else {
        return None;
    };
    Some(body.trim().to_string())
}

fn is_literal_receiver(kind: &str) -> bool {
    matches!(
        kind,
        "string"
            | "concatenated_string"
            | "integer"
            | "float"
            | "true"
            | "false"
            | "none"
            | "list"
            | "dictionary"
            | "tuple"
            | "set"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffers::{EDGE_KINDS, EDGE_ROW_SIZE, NODE_KINDS, NODE_ROW_SIZE, REF_ROW_SIZE};

    fn read_u32(buf: &[u8], off: usize) -> u32 {
        u32::from_le_bytes(buf[off..off + 4].try_into().unwrap())
    }

    fn read_str<'a>(arena: &'a [u8], buf: &[u8], off: usize) -> Option<&'a str> {
        let start = read_u32(buf, off);
        if start == NONE {
            return None;
        }
        let len = read_u32(buf, off + 4) as usize;
        let start = start as usize;
        std::str::from_utf8(&arena[start..start + len]).ok()
    }

    #[derive(Debug, PartialEq)]
    struct DecodedNode {
        kind: String,
        name: String,
        qualified_name: String,
        start_line: u32,
        end_line: u32,
    }

    fn decode_nodes(out: &EmitOut) -> Vec<DecodedNode> {
        let count = out.nodes.len() / NODE_ROW_SIZE;
        (0..count)
            .map(|i| {
                let row = &out.nodes[i * NODE_ROW_SIZE..(i + 1) * NODE_ROW_SIZE];
                DecodedNode {
                    kind: NODE_KINDS[row[0] as usize].to_string(),
                    name: read_str(&out.arena, row, 20).unwrap().to_string(),
                    qualified_name: read_str(&out.arena, row, 28).unwrap().to_string(),
                    start_line: read_u32(row, 4),
                    end_line: read_u32(row, 8),
                }
            })
            .collect()
    }

    fn decode_edges(out: &EmitOut) -> Vec<(u32, u32, String)> {
        let count = out.edges.len() / EDGE_ROW_SIZE;
        (0..count)
            .map(|i| {
                let row = &out.edges[i * EDGE_ROW_SIZE..(i + 1) * EDGE_ROW_SIZE];
                (
                    read_u32(row, 0),
                    read_u32(row, 4),
                    EDGE_KINDS[row[8] as usize].to_string(),
                )
            })
            .collect()
    }

    fn decode_refs(out: &EmitOut) -> Vec<(u32, String, String, u32)> {
        let count = out.refs.len() / REF_ROW_SIZE;
        (0..count)
            .map(|i| {
                let row = &out.refs[i * REF_ROW_SIZE..(i + 1) * REF_ROW_SIZE];
                (
                    read_u32(row, 0),
                    EDGE_KINDS[row[4] as usize].to_string(),
                    read_str(&out.arena, row, 16).unwrap().to_string(),
                    read_u32(row, 8),
                )
            })
            .collect()
    }

    #[test]
    fn extracts_python_classes_methods_functions_imports_inheritance_and_calls() {
        let source = r#"import os
from pkg.helpers import helper_fn as alias_fn

class BaseService:
    def base_run(self) -> None:
        pass

class Worker(BaseService):
    """Worker docstring."""
    def execute(self, item: str) -> int:
        self.base_run()
        helper_fn(item)
        client.send(item)
        return 1

def run_task() -> int:
    def inner():
        return 42
    w = Worker()
    return inner()
"#;
        let out = crate::extract("app.py", source, "python").expect("python extract");
        let nodes = decode_nodes(&out);
        let edges = decode_edges(&out);
        let refs = decode_refs(&out);

        assert!(nodes.iter().any(|n| n.kind == "file" && n.name == "app.py" && n.qualified_name == "app.py"));
        assert!(nodes.iter().any(|n| n.kind == "import" && n.name == "os"));
        assert!(nodes.iter().any(|n| n.kind == "import" && n.name == "pkg.helpers"));
        assert!(nodes.iter().any(|n| n.kind == "class" && n.name == "BaseService" && n.qualified_name == "BaseService"));
        assert!(nodes.iter().any(|n| n.kind == "method" && n.name == "base_run" && n.qualified_name == "BaseService::base_run"));
        assert!(nodes.iter().any(|n| n.kind == "class" && n.name == "Worker" && n.qualified_name == "Worker"));
        assert!(nodes.iter().any(|n| n.kind == "method" && n.name == "execute" && n.qualified_name == "Worker::execute"));
        assert!(nodes.iter().any(|n| n.kind == "function" && n.name == "run_task" && n.qualified_name == "run_task"));
        assert!(nodes.iter().any(|n| n.kind == "function" && n.name == "inner" && n.qualified_name == "run_task::inner"));

        // Verify `contains` edges exist from file -> classes/functions and class -> methods
        let worker_idx = nodes.iter().position(|n| n.name == "Worker").unwrap() as u32;
        let execute_idx = nodes.iter().position(|n| n.name == "execute").unwrap() as u32;
        assert!(edges.contains(&(0, worker_idx, "contains".to_string())));
        assert!(edges.contains(&(worker_idx, execute_idx, "contains".to_string())));

        // Verify inheritance (`extends` ref from Worker -> BaseService)
        assert!(refs.iter().any(|(from, kind, name, _)| *from == worker_idx && kind == "extends" && name == "BaseService"));

        // Verify imports (`imports` refs from file node)
        assert!(refs.iter().any(|(from, kind, name, _)| *from == 0 && kind == "imports" && name == "os"));
        assert!(refs.iter().any(|(from, kind, name, _)| *from == 0 && kind == "imports" && name == "pkg.helpers"));
        assert!(refs.iter().any(|(from, kind, name, _)| *from == 0 && kind == "imports" && name == "helper_fn"));

        // Verify calls (`calls` refs from execute -> base_run, helper_fn, client.send)
        assert!(refs.iter().any(|(from, kind, name, _)| *from == execute_idx && kind == "calls" && name == "base_run"));
        assert!(refs.iter().any(|(from, kind, name, _)| *from == execute_idx && kind == "calls" && name == "helper_fn"));
        assert!(refs.iter().any(|(from, kind, name, _)| *from == execute_idx && kind == "calls" && name == "client.send"));

        let run_task_idx = nodes.iter().position(|n| n.name == "run_task").unwrap() as u32;
        assert!(refs.iter().any(|(from, kind, name, _)| *from == run_task_idx && kind == "calls" && name == "Worker"));
        assert!(refs.iter().any(|(from, kind, name, _)| *from == run_task_idx && kind == "calls" && name == "inner"));
    }
}

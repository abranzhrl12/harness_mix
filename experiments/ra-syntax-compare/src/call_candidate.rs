//! Experimental RA equivalent of the bounded call facts in `index_rust.rs`.
//! Kept outside the runtime; intentionally preserves its conservative global
//! binding/import policy rather than attempting Rust name resolution.
use harness_domain::{Failure, execution::OperationContext};
use ra_ap_syntax::{Edition, SyntaxKind, SyntaxNode, ast::{self, AstNode, HasAttrs, HasDocComments, HasName, HasVisibility}};
use std::collections::{BTreeMap, BTreeSet};

const PARSER_LIMIT: Failure = ("PARSER_LIMIT", "Rust parser limited to 256 KiB");
const DEPTH_LIMIT: Failure = ("PARSER_LIMIT", "Conservative Rust nesting budget exceeded");
const SYNTAX_ERROR: Failure = ("SYNTAX_ERROR", "Rust source is not syntactically valid");

#[derive(Default)]
pub struct Facts {
    pub functions: BTreeMap<String, (i64, bool)>,
    pub modules: BTreeSet<String>,
    calls: BTreeMap<(String, i64, i64), Vec<String>>,
}

impl Facts {
    pub fn reserved(&self) -> usize {
        self.functions.keys().map(|s| 256 + s.capacity()).sum::<usize>()
            + self.modules.iter().map(|s| 256 + s.capacity()).sum::<usize>()
            + self.calls.iter().map(|((name, _, _), parts)| {
                256 + name.capacity() + parts.capacity() * std::mem::size_of::<String>()
                    + parts.iter().map(String::capacity).sum::<usize>()
            }).sum::<usize>()
    }

    pub fn call(&self, name: &str, line: i64, col: i64) -> Option<&[String]> {
        self.calls.get(&(name.to_owned(), line, col)).map(Vec::as_slice)
    }

    /// Test-only inspection surface for bidirectional parity with production's
    /// otherwise-private call map.
    pub fn call_entries(&self) -> impl Iterator<Item = (&str, i64, i64, &[String])> {
        self.calls.iter().map(|((name, line, col), parts)|
            (name.as_str(), *line, *col, parts.as_slice()))
    }
}

pub fn analyze(source: &str, context: &OperationContext) -> Result<Facts, Failure> {
    context.check()?;
    if source.len() > 256 * 1024 { return Err(PARSER_LIMIT); }
    let mut depth = 0usize;
    for (i, byte) in source.bytes().enumerate() {
        if i % 32768 == 0 { context.check()?; }
        if matches!(byte, b'(' | b'{' | b'[') { depth += 1; }
        else if matches!(byte, b')' | b'}' | b']') { depth = depth.saturating_sub(1); }
        if depth > 64 { return Err(DEPTH_LIMIT); }
    }
    let base = if source.starts_with('\u{feff}') { 3 } else { 0 };
    let input = &source[base..];
    let prepared = if input.starts_with("#!") && !input.starts_with("#![") {
        let end = input.find('\n').unwrap_or(input.len());
        format!("{}{}", " ".repeat(end), &input[end..])
    } else { input.to_owned() };
    let parsed = ast::SourceFile::parse(&prepared, Edition::Edition2024);
    if !parsed.errors().is_empty() { return Err(SYNTAX_ERROR); }
    context.check()?;
    let root = parsed.tree().syntax().clone();
    let mut facts = Facts::default();
    collect_top_level(&root, source, base, context, &mut facts)?;
    let bindings = bindings(&root, context)?;
    let mut receivers = BTreeSet::new();
    let function_names = facts.functions.keys().cloned().collect::<BTreeSet<_>>();
    visit_calls(&root, source, base, context, &bindings, &function_names, &mut receivers, &mut facts.calls)?;
    for receiver in &receivers { facts.calls.remove(receiver); }
    Ok(facts)
}

fn collect_top_level(root: &SyntaxNode, source: &str, base: usize,
    context: &OperationContext, facts: &mut Facts) -> Result<(), Failure> {
    for child in root.children() {
        context.check()?;
        if let Some(function) = ast::Fn::cast(child.clone()) {
            if only_doc_attrs(&function) {
                let Some(name) = function.name() else { continue; };
                let line = position(source, usize::from(function.fn_token().ok_or(SYNTAX_ERROR)?.text_range().start()) + base)
                    .ok_or(("RANGE", "Invalid Rust function position"))?.0;
                let visibility = function.visibility().map(|v| compact(v.syntax().text().to_string()));
                let public = matches!(visibility.as_deref(), Some("pub" | "pub(crate)" | "pub(super)" | "pub(incrate)" | "pub(insuper)"));
                facts.functions.insert(name.text().to_string(), (line, public));
            }
        } else if let Some(module) = ast::Module::cast(child) {
            let attrs = module.attrs().map(|a| compact(a.syntax().text().to_string())).collect::<Vec<_>>();
            let attrs_match = attrs.iter().all(|attr| attr == "#[cfg(windows)]" && cfg!(windows));
            if attrs_match && module.doc_comments().next().is_none() && module.item_list().is_none() {
                if let Some(name) = module.name() {
                    facts.modules.insert(name.text().to_string());
                }
            }
        }
    }
    Ok(())
}

fn only_doc_attrs(function: &ast::Fn) -> bool {
    function.attrs().all(|attr| {
        let text = compact(attr.syntax().text().to_string());
        text == "#[doc]" || text.starts_with("#[doc=") || text.starts_with("#[doc(")
    })
}

fn compact(text: String) -> String { text.chars().filter(|c| !c.is_whitespace()).collect() }

fn bindings(root: &SyntaxNode, context: &OperationContext) -> Result<Bindings, Failure> {
    let mut result = Bindings::default();
    collect_bindings(root, true, context, &mut result)?;
    Ok(result)
}

#[derive(Default)]
struct Bindings { names: BTreeSet<String>, imports: bool }

fn collect_bindings(node: &SyntaxNode, _root: bool, context: &OperationContext,
    out: &mut Bindings) -> Result<(), Failure> {
    context.check()?;
    for child in node.children() {
        context.check()?;
        if child.kind() == SyntaxKind::MODULE { continue; }
        match child.kind() {
            SyntaxKind::IDENT_PAT => if let Some(name) = ast::IdentPat::cast(child.clone()).and_then(|p| p.name()) {
                out.names.insert(name.text().to_string());
            },
            SyntaxKind::USE | SyntaxKind::MACRO_CALL | SyntaxKind::MACRO_RULES => out.imports = true,
            SyntaxKind::FN | SyntaxKind::CONST | SyntaxKind::STATIC
                if child.parent().is_some_and(|p| p.kind() == SyntaxKind::STMT_LIST) => {
                    let name = match child.kind() {
                        SyntaxKind::FN => ast::Fn::cast(child.clone()).and_then(|item| item.name()),
                        SyntaxKind::CONST => ast::Const::cast(child.clone()).and_then(|item| item.name()),
                        SyntaxKind::STATIC => ast::Static::cast(child.clone()).and_then(|item| item.name()),
                        _ => None,
                    };
                    if let Some(name) = name {
                        out.names.insert(name.text().to_string());
                    }
                }
            _ => {}
        }
        collect_bindings(&child, false, context, out)?;
    }
    Ok(())
}

fn visit_calls(node: &SyntaxNode, source: &str, base: usize,
    context: &OperationContext, bindings: &Bindings, functions: &BTreeSet<String>,
    receivers: &mut BTreeSet<(String, i64, i64)>,
    calls: &mut BTreeMap<(String, i64, i64), Vec<String>>) -> Result<(), Failure> {
    context.check()?;
    for child in node.children() {
        context.check()?;
        match child.kind() {
            SyntaxKind::MODULE | SyntaxKind::IMPL | SyntaxKind::TRAIT => continue,
            SyntaxKind::FN => {
                let Some(function) = ast::Fn::cast(child.clone()) else { continue; };
                if !only_doc_attrs(&function) { continue; }
            }
            SyntaxKind::METHOD_CALL_EXPR => {
                if let Some(method) = ast::MethodCallExpr::cast(child.clone()) {
                    if let Some(name) = method.name_ref() {
                        let at = usize::from(child.text_range().start()) + base;
                        if let Some((line, col)) = position(source, at) {
                            receivers.insert((name.text().to_string(), line, col));
                        }
                    }
                }
            }
            SyntaxKind::CALL_EXPR => {
                if let Some(call) = ast::CallExpr::cast(child.clone()) {
                    let path_expr = call.expr().and_then(|expr| match expr {
                        ast::Expr::PathExpr(path_expr) => Some(path_expr),
                        _ => None,
                    });
                    if let Some(path_expr) = path_expr {
                        if let Some(path) = path_expr.path() {
                            if !has_qualified_self(&path_expr.syntax()) {
                                let path_text = path.syntax().text().to_string();
                                if !path_text.starts_with("::") {
                                    let parts = path_parts(path);
                                    if let Some(first) = parts.first() {
                                        if !bindings.names.contains(first)
                                            && (parts.len() > 1 || (!bindings.imports && functions.contains(first))) {
                                            let at = usize::from(path_expr.syntax().text_range().start()) + base;
                                            if let Some((line, col)) = position(source, at) {
                                                calls.insert((parts.join("::"), line, col), parts);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        visit_calls(&child, source, base, context, bindings, functions, receivers, calls)?;
    }
    Ok(())
}

fn has_qualified_self(node: &SyntaxNode) -> bool {
    let text = node.text().to_string();
    text.trim_start().starts_with('<') || text.contains(" as ")
}

fn path_parts(path: ast::Path) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = Some(path);
    while let Some(path) = current {
        if let Some(segment) = path.segment() {
            if let Some(name) = segment.name_ref() {
                parts.push(name.text().to_string());
            }
        }
        current = path.qualifier();
    }
    parts.reverse();
    parts
}

fn position(source: &str, offset: usize) -> Option<(i64, i64)> {
    let prefix = source.get(..offset)?;
    Some((prefix.bytes().filter(|b| *b == b'\n').count() as i64 + 1,
        prefix.rsplit('\n').next()?.encode_utf16().count() as i64))
}

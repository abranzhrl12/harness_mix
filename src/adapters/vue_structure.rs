//! Bounded native structural parser. It builds SFC/template/style trees; it does
//! not emulate the Vue compiler, Sass expansion or template type checking.
use super::{project::Project, rust_symbols::Symbol, vue_syntax as lex};
use harness_domain::{
    Failure, contracts::VueExtractPreviewArgs, execution::OperationContext, hash, results::prefix,
};
use oxc_allocator::Allocator;
use oxc_ast::ast::{Argument, Expression, Statement};
use oxc_parser::Parser;
use oxc_span::SourceType;
use serde_json::{Value, json};
use std::{collections::BTreeSet, ops::Range, path::Path};

const SYNTAX: Failure = (
    "VUE_STRUCTURE",
    "Malformed or unsupported Vue structure; no source written",
);
const LIMIT: Failure = ("PARSER_LIMIT", "Vue structural parser budget exceeded");
const MAX_ITEMS: usize = 2000;

#[derive(Clone)]
struct Attr {
    name: String,
    value: String,
}
struct Block {
    kind: String,
    attrs: Vec<Attr>,
    range: Range<usize>,
    body: Range<usize>,
}
struct Element {
    tag: String,
    attrs: Vec<Attr>,
    range: Range<usize>,
    parent: Option<usize>,
    id: String,
}
struct Rule {
    selector: String,
    resolved: Vec<String>,
    range: Range<usize>,
    parent: Option<usize>,
    block: usize,
    uncertain: bool,
}
pub(super) struct Document {
    pub symbols: Vec<Symbol>,
    blocks: Vec<Block>,
    elements: Vec<Element>,
    rules: Vec<Rule>,
    expressions: Vec<(Range<usize>, String)>,
    notices: BTreeSet<String>,
}

fn line(source: &str, at: usize) -> usize {
    source.as_bytes()[..at]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
        + 1
}
fn attr<'a>(attrs: &'a [Attr], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(name))
        .map(|a| a.value.as_str())
}
fn attributes(text: &str) -> Result<Vec<Attr>, Failure> {
    let b = text.as_bytes();
    let mut p = 0;
    let mut out = Vec::new();
    while p < b.len() {
        while p < b.len() && (b[p].is_ascii_whitespace() || b[p] == b'/') {
            p += 1;
        }
        if p == b.len() {
            break;
        }
        let start = p;
        while p < b.len() && !b[p].is_ascii_whitespace() && !matches!(b[p], b'=' | b'/') {
            p += 1;
        }
        if start == p {
            return Err(SYNTAX);
        }
        let name = text[start..p].to_owned();
        while p < b.len() && b[p].is_ascii_whitespace() {
            p += 1;
        }
        let mut value = String::new();
        if b.get(p) == Some(&b'=') {
            p += 1;
            while p < b.len() && b[p].is_ascii_whitespace() {
                p += 1;
            }
            if p == b.len() {
                return Err(SYNTAX);
            }
            if matches!(b[p], b'\'' | b'"') {
                let quote = b[p];
                p += 1;
                let start = p;
                while p < b.len() && b[p] != quote {
                    p += 1;
                }
                if p == b.len() {
                    return Err(SYNTAX);
                }
                value = text[start..p].to_owned();
                p += 1;
            } else {
                let start = p;
                while p < b.len() && !b[p].is_ascii_whitespace() {
                    p += 1;
                }
                value = text[start..p].to_owned();
            }
        }
        out.push(Attr { name, value });
        if out.len() > 256 {
            return Err(LIMIT);
        }
    }
    Ok(out)
}
fn symbol(
    source: &str,
    digest: &str,
    name: String,
    container: String,
    kind: &'static str,
    range: Range<usize>,
    signature: String,
) -> Symbol {
    let id =
        hash(format!("{digest}:{kind}:{}:{}", range.start, range.end).as_bytes())[..24].to_owned();
    Symbol {
        name,
        container,
        kind,
        start: line(source, range.start),
        end: line(source, range.end.saturating_sub(1)),
        signature: prefix(&signature, 400).to_owned(),
        doc: String::new(),
        id,
        bytes: range,
    }
}
fn comment(source: &str, start: usize) -> Result<usize, Failure> {
    source[start + 4..]
        .find("-->")
        .map(|n| start + 4 + n + 3)
        .ok_or(SYNTAX)
}
fn is_void(tag: &str) -> bool {
    [
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
        "source", "track", "wbr",
    ]
    .contains(&tag)
}

pub(super) fn parse(
    source: &str,
    digest: &str,
    context: &OperationContext,
) -> Result<Document, Failure> {
    if source.len() > 512 * 1024 {
        return Err(LIMIT);
    }
    let mut doc = Document {
        symbols: vec![],
        blocks: vec![],
        elements: vec![],
        rules: vec![],
        expressions: vec![],
        notices: BTreeSet::new(),
    };
    let mut cursor = usize::from(source.starts_with('\u{feff}')) * 3;
    while cursor < source.len() {
        context.check()?;
        if source.as_bytes()[cursor].is_ascii_whitespace() {
            cursor += 1;
            continue;
        }
        if source[cursor..].starts_with("<!--") {
            cursor = comment(source, cursor)?;
            continue;
        }
        let start = cursor;
        let tag = lex::parse_tag(source, start).ok_or(SYNTAX)?;
        if tag.closing || doc.blocks.len() >= 64 {
            return Err(SYNTAX);
        }
        let kind = tag.name.to_ascii_lowercase();
        let attrs = attributes(tag.attrs)?;
        let body_start = tag.end;
        let (body_end, end) = if tag.self_closing {
            (body_start, body_start)
        } else if kind == "template"
            && attr(&attrs, "lang").is_none_or(|l| l == "html")
            && attr(&attrs, "src").is_none()
        {
            template(source, body_start, &mut doc, context)?
        } else {
            lex::closing_tag(source, body_start, &kind).ok_or(SYNTAX)?
        };
        if kind == "template" && doc.blocks.iter().any(|b| b.kind == "template") {
            return Err(SYNTAX);
        }
        let name = if kind == "script" && attr(&attrs, "setup").is_some() {
            "script setup"
        } else {
            &kind
        };
        if kind == "script"
            && doc.blocks.iter().any(|b| {
                b.kind == kind
                    && attr(&b.attrs, "setup").is_some() == attr(&attrs, "setup").is_some()
            })
        {
            return Err(SYNTAX);
        }
        doc.symbols.push(symbol(
            source,
            digest,
            name.to_owned(),
            String::new(),
            "vue_block",
            start..end,
            source[start..tag.end].to_owned(),
        ));
        let index = doc.blocks.len();
        doc.blocks.push(Block {
            kind: kind.clone(),
            attrs,
            range: start..end,
            body: body_start..body_end,
        });
        if attr(&doc.blocks[index].attrs, "src").is_some() {
            doc.notices
                .insert("External block source is not read or resolved".into());
        } else if kind == "script" {
            scripts(source, digest, index, &mut doc, context)?;
        } else if kind == "style" {
            styles(source, index, &mut doc, context)?;
        } else if kind == "template"
            && attr(&doc.blocks[index].attrs, "lang").is_some_and(|l| l != "html")
        {
            doc.notices
                .insert("Non-HTML template language is not structurally parsed".into());
        }
        cursor = end;
    }
    for (i, e) in doc.elements.iter_mut().enumerate() {
        let item = symbol(
            source,
            digest,
            format!("{}#{i}", e.tag),
            e.parent
                .map(|p| format!("template#{p}"))
                .unwrap_or_else(|| "template".into()),
            "vue_element",
            e.range.clone(),
            source[e.range.start..]
                .split('>')
                .next()
                .unwrap_or("")
                .to_owned(),
        );
        e.id = item.id.clone();
        doc.symbols.push(item);
    }
    for r in &doc.rules {
        doc.symbols.push(symbol(
            source,
            digest,
            r.selector.clone(),
            format!("style#{}", r.block),
            "vue_style_rule",
            r.range.clone(),
            format!("{} {{ … }}", r.selector),
        ));
    }
    if doc.symbols.len() > MAX_ITEMS {
        return Err(LIMIT);
    }
    doc.symbols.sort_by_key(|s| s.bytes.start);
    context.check()?;
    Ok(doc)
}

fn template(
    source: &str,
    from: usize,
    doc: &mut Document,
    context: &OperationContext,
) -> Result<(usize, usize), Failure> {
    let mut cursor = from;
    let mut stack: Vec<usize> = Vec::new();
    loop {
        context.check()?;
        let tag_at = source[cursor..].find('<').map(|n| cursor + n);
        let expr_at = source[cursor..].find("{{").map(|n| cursor + n);
        let start = [tag_at, expr_at]
            .into_iter()
            .flatten()
            .min()
            .ok_or(SYNTAX)?;
        if Some(start) == expr_at {
            let end = lex::skip_interpolation(source, start + 2).ok_or(SYNTAX)?;
            doc.expressions
                .push((start..end, source[start + 2..end - 2].to_owned()));
            cursor = end;
            continue;
        }
        if source[start..].starts_with("<!--") {
            cursor = comment(source, start)?;
            continue;
        }
        let tag = lex::parse_tag(source, start).ok_or(SYNTAX)?;
        if tag.closing {
            if stack.is_empty() && tag.name == "template" {
                return Ok((start, tag.end));
            }
            let index = stack.pop().ok_or(SYNTAX)?;
            if doc.elements[index].tag != tag.name {
                return Err(SYNTAX);
            }
            doc.elements[index].range.end = tag.end;
        } else {
            if doc.elements.len() >= MAX_ITEMS || stack.len() >= 128 {
                return Err(LIMIT);
            }
            if matches!(tag.name, "script" | "style") {
                return Err((
                    "VUE_STRUCTURE",
                    "Raw script/style inside template is unsupported",
                ));
            }
            let attrs = attributes(tag.attrs)?;
            for a in &attrs {
                if a.name.starts_with([':', '@', '#']) || a.name.starts_with("v-") {
                    doc.expressions.push((start..tag.end, a.value.clone()));
                }
            }
            let index = doc.elements.len();
            doc.elements.push(Element {
                tag: tag.name.to_owned(),
                attrs,
                range: start..tag.end,
                parent: stack.last().copied(),
                id: String::new(),
            });
            if !tag.self_closing && !is_void(tag.name) {
                stack.push(index);
            }
        }
        cursor = tag.end;
    }
}

fn scripts(
    source: &str,
    digest: &str,
    index: usize,
    doc: &mut Document,
    context: &OperationContext,
) -> Result<(), Failure> {
    let block = &doc.blocks[index];
    let lang = attr(&block.attrs, "lang").unwrap_or("js");
    let path = match lang {
        "js" | "javascript" => "sfc.js",
        "ts" | "typescript" => "sfc.ts",
        "jsx" => "sfc.jsx",
        "tsx" => "sfc.tsx",
        _ => {
            doc.notices.insert("Unsupported script language".into());
            return Ok(());
        }
    };
    let body = &source[block.body.clone()];
    let mut symbols = super::ts_symbols::collect(path, body, digest, context)?;
    let offset = block.body.start;
    let container = if attr(&block.attrs, "setup").is_some() {
        "script setup"
    } else {
        "script"
    };
    for item in &mut symbols {
        item.bytes = offset + item.bytes.start..offset + item.bytes.end;
        item.start = line(source, item.bytes.start);
        item.end = line(source, item.bytes.end.saturating_sub(1));
        item.container = format!("{container}/{}", item.container);
        item.id = hash(
            format!(
                "{digest}:{}:{}:{}",
                item.kind, item.bytes.start, item.bytes.end
            )
            .as_bytes(),
        )[..24]
            .to_owned();
    }
    doc.symbols.extend(symbols);
    let allocator = Allocator::default();
    let ast = Parser::new(
        &allocator,
        body,
        SourceType::from_path(path).unwrap_or_default(),
    )
    .parse();
    for stmt in &ast.program.body {
        context.check()?;
        match stmt {
            Statement::ImportDeclaration(decl) => {
                let range = offset + decl.span.start as usize..offset + decl.span.end as usize;
                doc.symbols.push(symbol(
                    source,
                    digest,
                    format!("import {}", decl.source.value),
                    container.into(),
                    "vue_import",
                    range.clone(),
                    source[range].to_owned(),
                ));
            }
            Statement::VariableDeclaration(decl) => {
                for d in &decl.declarations {
                    if let Some(Expression::CallExpression(call)) = &d.init {
                        macro_call(source, digest, offset, container, call, doc);
                    }
                }
            }
            Statement::ExpressionStatement(stmt) => {
                if let Expression::CallExpression(call) = &stmt.expression {
                    macro_call(source, digest, offset, container, call, doc);
                }
            }
            _ => {}
        }
    }
    Ok(())
}
fn macro_call(
    source: &str,
    digest: &str,
    offset: usize,
    container: &str,
    call: &oxc_ast::ast::CallExpression<'_>,
    doc: &mut Document,
) {
    if let Expression::Identifier(name) = &call.callee {
        if [
            "defineProps",
            "defineEmits",
            "defineModel",
            "defineSlots",
            "defineExpose",
            "defineOptions",
            "withDefaults",
        ]
        .contains(&name.name.as_str())
        {
            let range = offset + call.span.start as usize..offset + call.span.end as usize;
            doc.symbols.push(symbol(
                source,
                digest,
                name.name.to_string(),
                container.into(),
                "vue_macro",
                range.clone(),
                source[range].to_owned(),
            ));
        }
        if name.name == "withDefaults" {
            for argument in &call.arguments {
                if let Argument::CallExpression(inner) = argument {
                    macro_call(source, digest, offset, container, inner, doc);
                }
            }
        }
    }
}

// CSS/SCSS block parser with lexical states. Interpolation and Sass constructs
// are retained as uncertain; no expansion or property evaluation is attempted.
fn styles(
    source: &str,
    index: usize,
    doc: &mut Document,
    context: &OperationContext,
) -> Result<(), Failure> {
    let block = &doc.blocks[index];
    if !matches!(attr(&block.attrs, "lang").unwrap_or("css"), "css" | "scss") {
        doc.notices
            .insert("Only CSS/SCSS selectors are parsed".into());
        return Ok(());
    }
    let (mut p, end) = (block.body.start, block.body.end);
    let b = source.as_bytes();
    let mut begin = p;
    let mut stack: Vec<usize> = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut parens = 0usize;
    let mut brackets = 0usize;
    let mut interpolation = 0usize;
    while p < end {
        if p % 1024 == 0 {
            context.check()?;
        }
        let c = b[p];
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            p += 1;
            continue;
        }
        if c == b'/' && b.get(p + 1) == Some(&b'*') {
            let after = source[p + 2..end]
                .find("*/")
                .map(|n| p + 2 + n + 2)
                .ok_or(SYNTAX)?;
            if source[begin..p].trim().is_empty() {
                begin = after;
            }
            p = after;
            continue;
        }
        if c == b'/' && b.get(p + 1) == Some(&b'/') {
            let after = source[p..end].find('\n').map(|n| p + n + 1).unwrap_or(end);
            if source[begin..p].trim().is_empty() {
                begin = after;
            }
            p = after;
            continue;
        }
        if matches!(c, b'\'' | b'"') {
            quote = Some(c);
            p += 1;
            continue;
        }
        if c == b'#' && b.get(p + 1) == Some(&b'{') {
            interpolation += 1;
            p += 2;
            continue;
        }
        if interpolation > 0 {
            if c == b'{' {
                interpolation += 1;
            } else if c == b'}' {
                interpolation -= 1;
            }
            p += 1;
            continue;
        }
        match c {
            b'(' => parens += 1,
            b')' => {
                parens = parens.checked_sub(1).ok_or(SYNTAX)?;
            }
            b'[' => brackets += 1,
            b']' => {
                brackets = brackets.checked_sub(1).ok_or(SYNTAX)?;
            }
            b'{' if parens == 0 && brackets == 0 => {
                if doc.rules.len() >= MAX_ITEMS || stack.len() >= 128 {
                    return Err(LIMIT);
                }
                let leading = source[begin..p].len() - source[begin..p].trim_start().len();
                let start = begin + leading;
                let selector = source[start..p].trim().to_owned();
                if selector.is_empty() {
                    return Err(SYNTAX);
                }
                let parent = stack.last().copied();
                let uncertain = selector.contains("#{")
                    || selector.starts_with('@')
                    || selector.ends_with(':')
                    || selector.contains([':', '(', '[', '\\'])
                    || parent.is_some_and(|p| doc.rules[p].uncertain);
                let resolved = if let Some(parent) = parent {
                    let mut out = Vec::new();
                    for base in &doc.rules[parent].resolved {
                        for child in selector.split(',') {
                            if out.len() >= 128 {
                                return Err(LIMIT);
                            }
                            out.push(if child.contains('&') {
                                child.trim().replace('&', base)
                            } else {
                                format!("{base} {}", child.trim())
                            });
                        }
                    }
                    out
                } else {
                    selector.split(',').map(|s| s.trim().to_owned()).collect()
                };
                let i = doc.rules.len();
                doc.rules.push(Rule {
                    selector,
                    resolved,
                    range: start..p + 1,
                    parent,
                    block: index,
                    uncertain,
                });
                stack.push(i);
                begin = p + 1;
            }
            b'}' if parens == 0 && brackets == 0 => {
                let i = stack.pop().ok_or(SYNTAX)?;
                doc.rules[i].range.end = p + 1;
                begin = p + 1;
            }
            b';' if parens == 0 && brackets == 0 => begin = p + 1,
            _ => {}
        }
        p += 1;
    }
    if !stack.is_empty() || quote.is_some() || parens > 0 || brackets > 0 || interpolation > 0 {
        return Err(SYNTAX);
    }
    Ok(())
}
fn classes(selector: &str) -> BTreeSet<String> {
    let b = selector.as_bytes();
    let mut out = BTreeSet::new();
    let mut i = 0;
    let mut quote = None;
    while i < b.len() {
        if let Some(q) = quote {
            if b[i] == b'\\' {
                i += 1;
            } else if b[i] == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if matches!(b[i], b'\'' | b'"') {
            quote = Some(b[i]);
            i += 1;
            continue;
        }
        if b[i] == b'.' {
            let start = i + 1;
            i = start;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'-')) {
                i += 1;
            }
            if i > start {
                out.insert(selector[start..i].to_owned());
            }
        } else {
            i += 1;
        }
    }
    out
}
fn element_classes(element: &Element) -> BTreeSet<String> {
    attr(&element.attrs, "class")
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}
impl Document {
    pub(super) fn structure(&self, source: &str) -> Value {
        let blocks:Vec<_>=self.blocks.iter().map(|b|json!({"kind":b.kind,"start":line(source,b.range.start),"end":line(source,b.range.end.saturating_sub(1)),
            "lang":attr(&b.attrs,"lang"),"setup":attr(&b.attrs,"setup").is_some(),"scoped":attr(&b.attrs,"scoped").is_some(),"module":attr(&b.attrs,"module"),"external":attr(&b.attrs,"src").is_some()})).collect();
        let elements:Vec<_>=self.elements.iter().enumerate().map(|(i,e)|json!({"index":i,"symbolId":e.id,"tag":e.tag,"parent":e.parent,
            "start":line(source,e.range.start),"end":line(source,e.range.end.saturating_sub(1)),"classes":element_classes(e),
            "component":e.tag.chars().next().is_some_and(char::is_uppercase)||e.tag.contains('-'),
            "directives":e.attrs.iter().filter(|a|a.name.starts_with([':', '@', '#'])||a.name.starts_with("v-")).map(|a|json!({"name":a.name,"expression":a.value})).collect::<Vec<_>>() })).collect();
        let rules:Vec<_>=self.rules.iter().map(|r|{
            let names:BTreeSet<_>=r.resolved.iter().flat_map(|s|classes(s)).collect();
            let matching:Vec<_>=self.elements.iter().filter(|e|!element_classes(e).is_disjoint(&names)).map(|e|e.id.clone()).collect();
            json!({"selector":r.selector,"resolvedSelectors":r.resolved,"parent":r.parent,"styleBlock":r.block,"start":line(source,r.range.start),"end":line(source,r.range.end.saturating_sub(1)),
                "classes":names,"associatedElements":matching,"associationCertainty":if r.uncertain{"unknown"}else{"static_class_candidate"},"selectorMatchingPerformed":false})
        }).collect();
        json!({"parser":"native-vue-structural-subset-v1","blocks":blocks,"template":elements,"styles":rules,"limitations":self.notices,
            "notice":"Associations are static class candidates, not proof of CSS matching. Dynamic classes, Sass interpolation, scope and inheritance require review."})
    }
}

fn identifiers(text: &str) -> BTreeSet<String> {
    // Deliberately an over-approximation, never advertised as free-variable analysis.
    let mut out = BTreeSet::new();
    let b = text.as_bytes();
    let mut i = 0;
    let mut quote = None;
    while i < b.len() {
        if let Some(q) = quote {
            if b[i] == b'\\' {
                i += 1;
            } else if b[i] == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if matches!(b[i], b'\'' | b'"' | b'`') {
            quote = Some(b[i]);
            i += 1;
            continue;
        }
        if b[i].is_ascii_alphabetic() || matches!(b[i], b'_' | b'$') {
            let start = i;
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'$')) {
                i += 1;
            }
            let name = &text[start..i];
            if ![
                "true",
                "false",
                "null",
                "undefined",
                "this",
                "if",
                "else",
                "return",
                "in",
                "of",
                "typeof",
                "new",
                "const",
                "let",
                "Math",
                "JSON",
                "Number",
                "String",
                "Array",
                "Object",
                "$event",
            ]
            .contains(&name)
                && !text[..start].trim_end().ends_with('.')
            {
                out.insert(name.to_owned());
            }
        } else {
            i += 1;
        }
    }
    out
}

pub(super) fn extract_preview(
    project: &Project,
    args: &VueExtractPreviewArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    if args.expected_hash.len() != 64
        || !args.expected_hash.bytes().all(|b| b.is_ascii_hexdigit())
        || args.symbol_id.len() != 24
        || !args.symbol_id.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err((
            "ARGUMENT",
            "expectedHash SHA-256 and current template symbolId are required",
        ));
    }
    let name = &args.component_name;
    if name.len() > 100
        || !name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        || !name.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err((
            "ARGUMENT",
            "componentName must be a PascalCase ASCII identifier",
        ));
    }
    if Path::new(&args.path).extension().and_then(|s| s.to_str()) != Some("vue")
        || Path::new(&args.target_path)
            .extension()
            .and_then(|s| s.to_str())
            != Some("vue")
    {
        return Err(("ARGUMENT", "Both source and target must be Vue files"));
    }
    super::edit_commit::target(project, &args.path)?;
    let normalized = args.target_path.replace('\\', "/").to_ascii_lowercase();
    if normalized
        .split('/')
        .any(|p| [".git", ".hg", ".svn", ".agents", ".codegraph", "agents.md"].contains(&p))
        || super::project::external_restricted(&normalized)
        || super::project::external_sensitive(&normalized)
    {
        return Err(("PATH", "Protected extraction target"));
    }
    let target = project.target_optional(&args.target_path)?;
    if target
        .try_exists()
        .map_err(|_| ("PATH", "Cannot inspect target"))?
    {
        return Err(("TARGET_EXISTS", "Extraction target already exists"));
    }
    let bytes = project.bytes_with_context(&args.path, context)?;
    let digest = hash(&bytes);
    if !digest.eq_ignore_ascii_case(&args.expected_hash) {
        return Err(("STALE", "Vue source hash changed; request a new outline"));
    }
    let source =
        std::str::from_utf8(&bytes).map_err(|_| ("ENCODING", "Vue source must be UTF-8"))?;
    let doc = parse(source, &digest, context)?;
    let selected = doc
        .elements
        .iter()
        .find(|e| e.id == args.symbol_id)
        .ok_or((
            "STALE_SYMBOL",
            "Template element ID not found; request a new outline",
        ))?;
    if doc.symbols.iter().any(|s| s.name == *name)
        || doc
            .blocks
            .iter()
            .filter(|b| b.kind == "script")
            .any(|b| identifiers(&source[b.body.clone()]).contains(name))
    {
        return Err((
            "VUE_NAME_COLLISION",
            "Component name conflicts with a script identifier",
        ));
    }
    let mut issues = doc.notices.clone();
    let inside = |r: &Range<usize>| r.start >= selected.range.start && r.end <= selected.range.end;
    let selected_elements: Vec<_> = doc.elements.iter().filter(|e| inside(&e.range)).collect();
    let selected_classes: BTreeSet<_> = selected_elements
        .iter()
        .flat_map(|e| element_classes(e))
        .collect();
    let other_classes: BTreeSet<_> = doc
        .elements
        .iter()
        .filter(|e| !inside(&e.range))
        .flat_map(element_classes)
        .collect();
    let dynamic = doc.elements.iter().any(|e| {
        e.attrs
            .iter()
            .any(|a| matches!(a.name.as_str(), ":class" | "v-bind:class" | "v-bind"))
    });
    let mut deps = BTreeSet::new();
    let mut expressions = Vec::new();
    for (range, text) in &doc.expressions {
        if inside(range) {
            deps.extend(identifiers(text));
            expressions.push(text);
        }
    }
    if !expressions.is_empty() {
        issues.insert("Template expressions need lexical-scope, prop type and writable-binding review; suggested props are token candidates only".into());
    }
    for e in &selected_elements {
        if matches!(e.tag.as_str(), "slot" | "component") {
            issues.insert("Slots and dynamic components require component-boundary review".into());
        }
        if e.tag.chars().next().is_some_and(char::is_uppercase) || e.tag.contains('-') {
            issues.insert(format!(
                "Component import/registration must be resolved: {}",
                e.tag
            ));
        }
        for a in &e.attrs {
            if a.name == "ref"
                || a.name.starts_with("v-model")
                || a.name.starts_with("v-slot")
                || a.name.starts_with('#')
                || a.name == "v-for"
                || a.name == "v-html"
                || a.name == "v-pre"
            {
                issues.insert(format!(
                    "Review component-boundary semantics for {}",
                    a.name
                ));
            }
        }
    }
    if selected
        .attrs
        .iter()
        .any(|a| a.name == "v-else" || a.name == "v-else-if")
    {
        return Err((
            "VUE_BRANCH_BOUNDARY",
            "Cannot extract a dependent conditional branch alone",
        ));
    }
    // Also reject an if branch whose following sibling depends on it.
    if doc.elements.iter().any(|e| {
        e.parent == selected.parent
            && e.range.start >= selected.range.end
            && source[selected.range.end..e.range.start].trim().is_empty()
            && e.attrs
                .iter()
                .any(|a| a.name == "v-else" || a.name == "v-else-if")
    }) {
        return Err((
            "VUE_BRANCH_BOUNDARY",
            "Conditional chain must be reviewed/extracted together",
        ));
    }
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let mut moved = Vec::new();
    let mut retained = Vec::new();
    let mut child_styles = String::new();
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    for (block_index, block) in doc
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| b.kind == "style")
    {
        let mut pieces = Vec::new();
        for root in doc
            .rules
            .iter()
            .filter(|r| r.block == block_index && r.parent.is_none())
        {
            let subtree: Vec<_> = doc
                .rules
                .iter()
                .filter(|r| r.range.start >= root.range.start && r.range.end <= root.range.end)
                .collect();
            let names: BTreeSet<_> = subtree
                .iter()
                .flat_map(|r| r.resolved.iter().flat_map(|s| classes(s)))
                .collect();
            if names.is_disjoint(&selected_classes) {
                continue;
            }
            let text = &source[root.range.clone()];
            let uncertain = dynamic
                || subtree.iter().any(|r| r.uncertain)
                || text.contains('$')
                || text.contains('@')
                || text.contains("var(")
                || text.contains("v-bind(")
                || attr(&block.attrs, "module").is_some()
                || text.contains("url(")
                || text.contains("animation")
                || (attr(&block.attrs, "lang") == Some("scss")
                    && (text.contains('(') || source[block.body.clone()].contains('@')))
                || root
                    .selector
                    .split(',')
                    .any(|s| !s.trim().starts_with('.') || classes(s).is_empty());
            let exclusive = !names.is_empty()
                && names.is_subset(&selected_classes)
                && names.is_disjoint(&other_classes);
            if exclusive && !uncertain {
                pieces.push(text);
                moved.push(root.selector.clone());
                edits.push((root.range.clone(), String::new()));
            } else {
                retained.push(root.selector.clone());
                issues.insert("Shared, dynamic or context-dependent style rules remain in the parent; scoped styling across the new boundary requires review".into());
            }
        }
        if !pieces.is_empty() {
            child_styles.push_str(newline);
            child_styles.push_str(&source[block.range.start..block.body.start]);
            child_styles.push_str(newline);
            child_styles.push_str(&pieces.join(newline));
            child_styles.push_str(newline);
            child_styles.push_str("</style>");
            child_styles.push_str(newline);
        }
    }
    if dynamic {
        issues.insert("Dynamic classes prevent proving BEM rule exclusivity".into());
    }
    let parent_dir = Path::new(&args.path).parent().unwrap_or(Path::new(""));
    let target_parts: Vec<_> = Path::new(&args.target_path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let parent_parts: Vec<_> = parent_dir
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let common = parent_parts
        .iter()
        .zip(&target_parts)
        .take_while(|(a, b)| a == b)
        .count();
    let import_path = format!(
        "{}{}",
        "../".repeat(parent_parts.len() - common),
        target_parts[common..].join("/")
    );
    let import_path = if import_path.starts_with('.') {
        import_path
    } else {
        format!("./{import_path}")
    };
    let import = format!(
        "{newline}import {name} from {};{newline}",
        json!(import_path)
    );
    if let Some(setup) = doc
        .blocks
        .iter()
        .find(|b| b.kind == "script" && attr(&b.attrs, "setup").is_some())
    {
        if attr(&setup.attrs, "src").is_some() {
            return Err((
                "VUE_EXTERNAL_SCRIPT",
                "External setup scripts cannot be rewritten in this preview",
            ));
        }
        edits.push((setup.body.start..setup.body.start, import));
    } else {
        edits.push((0..0, format!("<script setup>{import}</script>{newline}")));
    }
    let bindings = deps
        .iter()
        .map(|d| format!(" :{d}=\"{d}\""))
        .collect::<String>();
    let replacement = format!("<{name}{bindings} />");
    edits.push((selected.range.clone(), replacement));
    edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    let mut parent = source.to_owned();
    for (range, text) in edits {
        parent.replace_range(range, &text);
    }
    let props = if deps.is_empty() {
        String::new()
    } else {
        format!(
            "<script setup>{newline}defineProps({});{newline}</script>{newline}{newline}",
            json!(deps)
        )
    };
    let child = format!(
        "{props}<template>{newline}{}{newline}</template>{newline}{child_styles}",
        &source[selected.range.clone()]
    );
    super::vue_syntax::validate(&args.path, &parent, context)?;
    super::vue_syntax::validate(&args.target_path, &child, context)?;
    parse(&parent, &hash(parent.as_bytes()), context)?;
    parse(&child, &hash(child.as_bytes()), context)?;
    // Optimistic planning also rechecks both observations; application will need
    // its own multi-file commit/recovery design and fresh checks.
    if hash(&project.bytes_with_context(&args.path, context)?) != digest {
        return Err(("STALE", "Vue source changed during preview"));
    }
    if target
        .try_exists()
        .map_err(|_| ("PATH", "Cannot recheck target"))?
    {
        return Err(("TARGET_EXISTS", "Target appeared during preview"));
    }
    let patch = format!(
        "{}{}",
        super::edit::preview_diff(&args.path, source, &parent),
        super::edit::preview_diff(&args.target_path, "", &child)
    );
    let files = json!([{"path":args.path,"operation":"replace","baseHash":digest,"candidateHash":hash(parent.as_bytes()),"content":parent},
        {"path":args.target_path,"operation":"create","expectedAbsent":true,"candidateHash":hash(child.as_bytes()),"content":child}]);
    Ok(
        json!({"path":args.path,"baseHash":digest,"symbolId":selected.id,"targetPath":args.target_path,
        "previewOnly":true,"writesPerformed":false,"applicationSupported":false,"reviewRequired":!issues.is_empty(),
        "interfaceProposal":{"props":deps,"emits":[],"certainty":"lexical_candidates","note":"Event handlers are proposed as callback props; events, scopes, v-model, slots and refs require review"},
        "styles":{"movedRules":moved,"retainedRules":retained,"strategy":"whole exclusive static rule trees; shared and uncertain rules retained"},
        "unresolved":issues,"syntaxValidation":"structural_subset_and_embedded_scripts","patch":prefix(&patch,12000),"truncated":patch.len()>12000,
        "notice":"Read-only proposal. No automatic multi-file application or rollback is provided. Review unresolved dependencies and complete file content before applying.",
        "_detail":{"files":files,"patch":patch}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    const SOURCE: &str = "<script setup lang=\"ts\">\nimport Icon from './Icon.vue'\nconst props = withDefaults(defineProps<{ title: string }>(), { title: 'Hi' })\nconst emit = defineEmits(['save'])\nfunction save() { emit('save') }\n</script>\n<template>\n  <main class=\"page\">\n    <section class=\"card\"><h2 class=\"card__title\">Hello</h2></section>\n    <aside class=\"other\">Other</aside>\n  </main>\n</template>\n<style scoped lang=\"scss\">\n.card { color: red; &__title { font-weight: bold; } }\n.other { color: blue; }\n</style>\n";

    fn preview(source: &str) -> (tempfile::TempDir, VueExtractPreviewArgs) {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("Parent.vue"), source).unwrap();
        let document = parse(
            source,
            &hash(source.as_bytes()),
            &OperationContext::default(),
        )
        .unwrap();
        let selected = document
            .elements
            .iter()
            .find(|e| e.tag == "section")
            .unwrap();
        let args = VueExtractPreviewArgs {
            path: "Parent.vue".into(),
            expected_hash: hash(source.as_bytes()),
            symbol_id: selected.id.clone(),
            target_path: "CardView.vue".into(),
            component_name: "CardView".into(),
            session: None,
        };
        (root, args)
    }

    #[test]
    fn structure_reports_exact_script_ranges_template_hierarchy_and_nested_bem() {
        let document = parse(
            SOURCE,
            &hash(SOURCE.as_bytes()),
            &OperationContext::default(),
        )
        .unwrap();
        let structure = document.structure(SOURCE);
        assert_eq!(structure["blocks"][0]["setup"], true);
        assert_eq!(structure["blocks"][0]["lang"], "ts");
        assert_eq!(structure["blocks"][2]["scoped"], true);
        assert_eq!(structure["template"][1]["parent"], 0);
        assert!(document.symbols.iter().any(|s| s.kind == "vue_import"));
        for name in ["defineProps", "withDefaults", "defineEmits", "save"] {
            let item = document.symbols.iter().find(|s| s.name == name).unwrap();
            assert!(SOURCE[item.bytes.clone()].contains(name));
            assert_eq!(item.start, line(SOURCE, item.bytes.start));
        }
        assert_eq!(
            structure["styles"][1]["resolvedSelectors"],
            json!([".card__title"])
        );
        assert_eq!(structure["styles"][1]["classes"], json!(["card__title"]));
        assert_eq!(
            structure["styles"][1]["associationCertainty"],
            "static_class_candidate"
        );
        assert_eq!(
            structure["styles"][1]["associatedElements"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn preview_moves_exclusive_bem_as_a_whole_and_never_writes() {
        let (root, args) = preview(SOURCE);
        let project = Project::new(root.path()).unwrap();
        let result = extract_preview(&project, &args, &OperationContext::default()).unwrap();
        assert_eq!(result["writesPerformed"], false);
        assert_eq!(result["reviewRequired"], false);
        assert_eq!(result["styles"]["movedRules"], json!([".card"]));
        let files = &result["_detail"]["files"];
        let parent = files[0]["content"].as_str().unwrap();
        let child = files[1]["content"].as_str().unwrap();
        assert!(parent.contains("import CardView from \"./CardView.vue\""));
        assert!(parent.contains("<CardView />"));
        assert!(!parent.contains(".card {"));
        assert!(parent.contains(".other {"));
        assert!(child.contains("<section class=\"card\""));
        assert!(child.contains("&__title"));
        assert!(child.contains("<style scoped lang=\"scss\">"));
        assert_eq!(
            std::fs::read_to_string(root.path().join("Parent.vue")).unwrap(),
            SOURCE
        );
        assert!(!root.path().join("CardView.vue").exists());
    }

    #[test]
    fn dynamic_expressions_shared_rules_slots_refs_and_models_require_review() {
        let source = "<template><main><section class=\"card\" :class=\"variant\"><input ref=\"field\" v-model=\"value\"/><slot>{{ title }}</slot></section><aside class=\"card\"/></main></template><style scoped lang=\"scss\">.card { color: red; }</style>";
        let (root, args) = preview(source);
        let result = extract_preview(
            &Project::new(root.path()).unwrap(),
            &args,
            &OperationContext::default(),
        )
        .unwrap();
        assert_eq!(result["reviewRequired"], true);
        assert!(
            result["interfaceProposal"]["props"]
                .as_array()
                .unwrap()
                .contains(&json!("title"))
        );
        assert_eq!(result["styles"]["movedRules"], json!([]));
        assert_eq!(result["styles"]["retainedRules"], json!([".card"]));
        let unresolved = result["unresolved"].to_string();
        for text in ["v-model", "ref", "Slots", "Dynamic classes"] {
            assert!(unresolved.contains(text), "{unresolved}");
        }
    }

    #[test]
    fn style_contexts_and_interpolated_selectors_are_not_moved_as_exclusive() {
        let source = "<template><section class=\"card\">Hello</section></template><style lang=\"scss\">$tone: red; .card { color: $tone; } @media (width > 10px) { .card { color: blue; } } .#{$name} { color: red; }</style>";
        let (root, args) = preview(source);
        let doc = parse(source, &args.expected_hash, &OperationContext::default()).unwrap();
        let s = doc.structure(source);
        assert!(
            s["styles"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["associationCertainty"] == "unknown")
        );
        let result = extract_preview(
            &Project::new(root.path()).unwrap(),
            &args,
            &OperationContext::default(),
        )
        .unwrap();
        assert_eq!(result["styles"]["movedRules"], json!([]));
        assert_eq!(result["reviewRequired"], true);
    }

    #[test]
    fn stale_sources_existing_or_protected_targets_and_branch_splits_are_refused() {
        let (root, mut args) = preview(SOURCE);
        let project = Project::new(root.path()).unwrap();
        args.expected_hash = "0".repeat(64);
        assert_eq!(
            extract_preview(&project, &args, &OperationContext::default())
                .unwrap_err()
                .0,
            "STALE"
        );
        args.expected_hash = hash(SOURCE.as_bytes());
        std::fs::write(root.path().join("CardView.vue"), "keep").unwrap();
        assert_eq!(
            extract_preview(&project, &args, &OperationContext::default())
                .unwrap_err()
                .0,
            "TARGET_EXISTS"
        );
        args.target_path = "../Outside.vue".into();
        assert!(extract_preview(&project, &args, &OperationContext::default()).is_err());
        args.target_path = ".agents/CardView.vue".into();
        assert_eq!(
            extract_preview(&project, &args, &OperationContext::default())
                .unwrap_err()
                .0,
            "PATH"
        );
        let source = "<template><main><section v-if=\"show\">A</section><aside v-else>B</aside></main></template>";
        let (root, args) = preview(source);
        assert_eq!(
            extract_preview(
                &Project::new(root.path()).unwrap(),
                &args,
                &OperationContext::default()
            )
            .unwrap_err()
            .0,
            "VUE_BRANCH_BOUNDARY"
        );
    }

    #[test]
    fn quoted_tags_comments_unicode_crlf_and_cancellation_preserve_ranges() {
        let source = "<!-- 😀 <script>fake</script> -->\r\n<script setup>const label = 'á😀'</script>\r\n<template><section title=\">\">{{ label + '</template>' }}<!-- <x/> --><br/></section></template>";
        let (root, args) = preview(source);
        let result = extract_preview(
            &Project::new(root.path()).unwrap(),
            &args,
            &OperationContext::default(),
        )
        .unwrap();
        let child = result["_detail"]["files"][1]["content"].as_str().unwrap();
        assert!(child.contains("\r\n"));
        assert!(child.contains("{{ label + '</template>' }}"));
        let context = OperationContext::default();
        context.cancel();
        assert_eq!(
            parse(source, &args.expected_hash, &context)
                .err()
                .unwrap()
                .0,
            "CANCELLED"
        );
        assert!(
            parse(
                "<template><div></template>",
                "x",
                &OperationContext::default()
            )
            .is_err()
        );
        assert!(parse("<style>.x {</style>", "x", &OperationContext::default()).is_err());
    }
}

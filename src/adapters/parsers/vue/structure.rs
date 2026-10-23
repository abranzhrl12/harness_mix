//! Bounded native structural parser. It builds SFC/template/style trees; it does
//! not emulate the Vue compiler, Sass expansion or template type checking.
use crate::adapters::parsers::{model::Symbol, vue::syntax as lex};
use harness_domain::{Failure, execution::OperationContext, hash, results::prefix};
use oxc_allocator::Allocator;
use oxc_ast::ast::{Argument, Expression, Statement};
use oxc_parser::Parser;
use oxc_span::SourceType;
use serde_json::{Value, json};
use std::{collections::BTreeSet, ops::Range};

const SYNTAX: Failure = (
    "VUE_STRUCTURE",
    "Malformed or unsupported Vue structure; no source written",
);
const LIMIT: Failure = ("PARSER_LIMIT", "Vue structural parser budget exceeded");
const MAX_ITEMS: usize = 2000;

#[derive(Clone)]
pub(in crate::adapters) struct Attr {
    pub(in crate::adapters) name: String,
    pub(in crate::adapters) value: String,
}
pub(in crate::adapters) struct Block {
    pub(in crate::adapters) kind: String,
    pub(in crate::adapters) attrs: Vec<Attr>,
    pub(in crate::adapters) range: Range<usize>,
    pub(in crate::adapters) body: Range<usize>,
}
pub(in crate::adapters) struct Element {
    pub(in crate::adapters) tag: String,
    pub(in crate::adapters) attrs: Vec<Attr>,
    pub(in crate::adapters) range: Range<usize>,
    pub(in crate::adapters) parent: Option<usize>,
    pub(in crate::adapters) id: String,
}
pub(in crate::adapters) struct Rule {
    pub(in crate::adapters) selector: String,
    pub(in crate::adapters) resolved: Vec<String>,
    pub(in crate::adapters) range: Range<usize>,
    pub(in crate::adapters) parent: Option<usize>,
    pub(in crate::adapters) block: usize,
    pub(in crate::adapters) uncertain: bool,
}
pub(in crate::adapters) struct Document {
    pub(in crate::adapters) symbols: Vec<Symbol>,
    pub(in crate::adapters) blocks: Vec<Block>,
    pub(in crate::adapters) elements: Vec<Element>,
    pub(in crate::adapters) rules: Vec<Rule>,
    pub(in crate::adapters) expressions: Vec<(Range<usize>, String)>,
    pub(in crate::adapters) notices: BTreeSet<String>,
}

fn line(source: &str, at: usize) -> usize {
    source.as_bytes()[..at]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
        + 1
}
pub(in crate::adapters) fn attr<'a>(attrs: &'a [Attr], name: &str) -> Option<&'a str> {
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
        attributes: Vec::new(),
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

pub(in crate::adapters) fn parse(
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
            format!("{} {{ â€¦ }}", r.selector),
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
    let mut symbols =
        crate::adapters::parsers::ecmascript::symbols::collect(path, body, digest, context)?;
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
                    format!("import {}", decl.source.value.display()),
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
pub(in crate::adapters) fn classes(selector: &str) -> BTreeSet<String> {
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
pub(in crate::adapters) fn element_classes(element: &Element) -> BTreeSet<String> {
    attr(&element.attrs, "class")
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}
impl Document {
    pub(in crate::adapters) fn structure(&self, source: &str) -> Value {
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

#[cfg(test)]
#[path = "structure_tests.rs"]
mod tests;

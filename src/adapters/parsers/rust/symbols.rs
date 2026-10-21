//! Declaration navigation only. Never infer callers or expand macros.
use crate::adapters::parsers::rust::syntax as rust_syntax;
use harness_domain::{Failure, execution::OperationContext, hash, results::prefix};
use proc_macro2::Span;
use syn::{Attribute, ImplItem, Item, TraitItem, spanned::Spanned};

use super::super::model::Symbol;

struct Definition<'a> {
    name: String,
    kind: &'static str,
    span: Span,
    attrs: &'a [Attribute],
    body: Option<Span>,
}
struct Collector<'a> {
    source: &'a str,
    digest: &'a str,
    base: usize,
    context: &'a OperationContext,
    symbols: Vec<Symbol>,
}

impl Collector<'_> {
    fn range(&self, span: Span) -> Result<std::ops::Range<usize>, Failure> {
        let range = span.byte_range();
        let range = range.start + self.base..range.end + self.base;
        if range.start >= range.end || self.source.get(range.clone()).is_none() {
            return Err(("RANGE", "Parser offsets incompatible with source"));
        }
        Ok(range)
    }
    fn text(&self, span: Span) -> Result<&str, Failure> {
        Ok(&self.source[self.range(span)?])
    }
    fn record(&mut self, definition: Definition<'_>, container: &str) -> Result<String, Failure> {
        self.context.check()?;
        if self.symbols.len() >= 1000 || definition.name.len() > 1024 || container.len() > 4096 {
            return Err(("PARSER_LIMIT", "Symbol count or identifier budget exceeded"));
        }
        let bytes = self.range(definition.span)?;
        let signature_start = definition
            .attrs
            .last()
            .map(|attr| self.range(attr.span()).map(|r| r.end))
            .transpose()?
            .unwrap_or(bytes.start);
        let signature_end = definition
            .body
            .map(|span| self.range(span).map(|r| r.start))
            .transpose()?
            .unwrap_or(bytes.end);
        let signature_text = self
            .source
            .get(signature_start..signature_end)
            .ok_or(("RANGE", "Invalid signature offsets"))?;
        let compact = signature_text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let signature = if definition.body.is_some() {
            format!("{} …", prefix(&compact, 398))
        } else {
            prefix(&compact, 400).to_owned()
        };
        let doc = definition
            .attrs
            .iter()
            .filter_map(|attr| {
                if !attr.path().is_ident("doc") {
                    return None;
                }
                let syn::Meta::NameValue(value) = &attr.meta else {
                    return None;
                };
                let syn::Expr::Lit(literal) = &value.value else {
                    return None;
                };
                let syn::Lit::Str(text) = &literal.lit else {
                    return None;
                };
                Some(text.value())
            })
            .collect::<Vec<_>>()
            .join("\n");
        // Node's offsets are UTF-16; keep its ID digest convention, not byte offsets.
        let from = self.source[..bytes.start].encode_utf16().count();
        let to = from + self.source[bytes.clone()].encode_utf16().count();
        let id = hash(format!("{}:{from}:{to}", self.digest).as_bytes())[..24].to_owned();
        let start = self.source[..bytes.start]
            .bytes()
            .filter(|b| *b == b'\n')
            .count()
            + 1;
        let end = self.source[..bytes.end]
            .bytes()
            .filter(|b| *b == b'\n')
            .count()
            + usize::from(!self.source[..bytes.end].ends_with('\n'));
        let child = if container.is_empty() {
            definition.name.clone()
        } else {
            format!("{container}::{}", definition.name)
        };
        self.symbols.push(Symbol {
            name: definition.name,
            container: container.into(),
            kind: definition.kind,
            start,
            end,
            signature,
            doc: prefix(&doc, 600).into(),
            id,
            bytes,
        });
        Ok(child)
    }
    fn items(&mut self, items: &[Item], container: &str) -> Result<(), Failure> {
        for item in items {
            self.context.check()?;
            let (name, kind, attrs, body) = match item {
                Item::Fn(n) => (
                    n.sig.ident.to_string(),
                    "function_item",
                    &n.attrs,
                    Some(n.block.brace_token.span.open()),
                ),
                Item::Struct(n) => (
                    n.ident.to_string(),
                    "struct_item",
                    &n.attrs,
                    match &n.fields {
                        syn::Fields::Named(fields) => Some(fields.brace_token.span.open()),
                        _ => None,
                    },
                ),
                Item::Enum(n) => (
                    n.ident.to_string(),
                    "enum_item",
                    &n.attrs,
                    Some(n.brace_token.span.open()),
                ),
                Item::Union(n) => (
                    n.ident.to_string(),
                    "union_item",
                    &n.attrs,
                    Some(n.fields.brace_token.span.open()),
                ),
                Item::Trait(n) => (
                    n.ident.to_string(),
                    "trait_item",
                    &n.attrs,
                    Some(n.brace_token.span.open()),
                ),
                Item::TraitAlias(n) => (n.ident.to_string(), "trait_item", &n.attrs, None),
                Item::Mod(n) => (
                    n.ident.to_string(),
                    "mod_item",
                    &n.attrs,
                    n.content.as_ref().map(|(brace, _)| brace.span.open()),
                ),
                Item::Type(n) => (n.ident.to_string(), "type_item", &n.attrs, None),
                Item::Const(n) => (n.ident.to_string(), "const_item", &n.attrs, None),
                Item::Static(n) => (n.ident.to_string(), "static_item", &n.attrs, None),
                Item::Macro(n) if n.ident.is_some() => (
                    n.ident.as_ref().unwrap().to_string(),
                    "macro_definition",
                    &n.attrs,
                    Some(n.mac.delimiter.span().open()),
                ),
                Item::Impl(n) => {
                    let ty = self.text(n.self_ty.span())?;
                    let name = if let Some((negative, path, _)) = &n.trait_ {
                        format!(
                            "impl {}{} for {ty}",
                            if negative.is_some() { "!" } else { "" },
                            self.text(path.span())?
                        )
                    } else {
                        format!("impl {ty}")
                    };
                    (name, "impl_item", &n.attrs, Some(n.brace_token.span.open()))
                }
                _ => continue,
            };
            let child = self.record(
                Definition {
                    name,
                    kind,
                    attrs,
                    body,
                    span: item.span(),
                },
                container,
            )?;
            match item {
                Item::Mod(n) => {
                    if let Some((_, items)) = &n.content {
                        self.items(items, &child)?;
                    }
                }
                Item::Impl(n) => {
                    for member in &n.items {
                        let (name, kind, attrs, body) = match member {
                            ImplItem::Fn(m) => (
                                m.sig.ident.to_string(),
                                "function_item",
                                &m.attrs,
                                Some(m.block.brace_token.span.open()),
                            ),
                            ImplItem::Const(m) => {
                                (m.ident.to_string(), "const_item", &m.attrs, None)
                            }
                            ImplItem::Type(m) => (m.ident.to_string(), "type_item", &m.attrs, None),
                            _ => continue,
                        };
                        self.record(
                            Definition {
                                name,
                                kind,
                                attrs,
                                body,
                                span: member.span(),
                            },
                            &child,
                        )?;
                    }
                }
                Item::Trait(n) => {
                    for member in &n.items {
                        let (name, kind, attrs, body) = match member {
                            TraitItem::Fn(m) => (
                                m.sig.ident.to_string(),
                                if m.default.is_some() {
                                    "function_item"
                                } else {
                                    "function_signature_item"
                                },
                                &m.attrs,
                                m.default
                                    .as_ref()
                                    .map(|block| block.brace_token.span.open()),
                            ),
                            TraitItem::Const(m) => {
                                (m.ident.to_string(), "const_item", &m.attrs, None)
                            }
                            TraitItem::Type(m) => {
                                (m.ident.to_string(), "type_item", &m.attrs, None)
                            }
                            _ => continue,
                        };
                        self.record(
                            Definition {
                                name,
                                kind,
                                attrs,
                                body,
                                span: member.span(),
                            },
                            &child,
                        )?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

pub(in crate::adapters) fn collect(
    source: &str,
    digest: &str,
    context: &OperationContext,
) -> Result<Vec<Symbol>, Failure> {
    rust_syntax::with_ast(source, context, |ast, base| {
        let mut collector = Collector {
            source,
            digest,
            base,
            context,
            symbols: Vec::new(),
        };
        collector.items(&ast.items, "")?;
        Ok(collector.symbols)
    })
}

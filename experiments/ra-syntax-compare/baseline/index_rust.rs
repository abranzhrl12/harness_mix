//! Deliberately bounded Rust free-function proofs; no receiver/type guessing.
use super::{Failure, OperationContext};
use std::collections::{BTreeMap, BTreeSet};
use syn::{Item, spanned::Spanned, visit::Visit};

struct Positions<'a> {
    source: &'a str,
    starts: Vec<usize>,
}
impl<'a> Positions<'a> {
    fn new(source: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(
            source
                .bytes()
                .enumerate()
                .filter_map(|(i, b)| (b == b'\n').then_some(i + 1)),
        );
        Self { source, starts }
    }
    fn get(&self, offset: usize) -> Option<(i64, i64)> {
        if !self.source.is_char_boundary(offset) {
            return None;
        }
        let line = self
            .starts
            .partition_point(|start| *start <= offset)
            .checked_sub(1)?;
        let column = self
            .source
            .get(self.starts[line]..offset)?
            .encode_utf16()
            .count();
        Some((line as i64 + 1, column as i64))
    }
}

#[derive(Default)]
pub(super) struct Facts {
    pub functions: BTreeMap<String, (i64, bool)>,
    pub modules: BTreeSet<String>,
    calls: BTreeMap<(String, i64, i64), Vec<String>>,
}
impl Facts {
    pub fn reserved(&self) -> usize {
        self.functions
            .keys()
            .map(|s| 256 + s.capacity())
            .sum::<usize>()
            + self
                .modules
                .iter()
                .map(|s| 256 + s.capacity())
                .sum::<usize>()
            + self
                .calls
                .iter()
                .map(|((name, _, _), parts)| {
                    256 + name.capacity()
                        + parts.capacity() * std::mem::size_of::<String>()
                        + parts.iter().map(String::capacity).sum::<usize>()
                })
                .sum::<usize>()
    }
    pub fn call(&self, name: &str, line: i64, col: i64) -> Option<&[String]> {
        self.calls.get(&(name.into(), line, col)).map(Vec::as_slice)
    }
}

pub(super) fn analyze(source: &str, context: &OperationContext) -> Result<Facts, Failure> {
    super::super::rust_syntax::with_ast(source, context, |file, base| {
        let mut facts = Facts::default();
        for item in &file.items {
            match item {
                Item::Fn(f) if f.attrs.iter().all(|a| a.path().is_ident("doc")) => {
                    let public = matches!(f.vis, syn::Visibility::Public(_))
                        || matches!(&f.vis, syn::Visibility::Restricted(v) if v.path.is_ident("crate") || v.path.is_ident("super"));
                    facts.functions.insert(f.sig.ident.to_string(), (f.sig.fn_token.span.start().line as i64, public));
                }
                Item::Mod(m) if m.attrs.iter().all(|a| matches!(&a.meta, syn::Meta::List(list) if list.path.is_ident("cfg") && list.tokens.to_string() == "windows" && cfg!(windows))) && m.content.is_none() => {
                    facts.modules.insert(m.ident.to_string());
                }
                _ => {}
            }
        }
        // Overblocking across scopes is intentional: uncertain shadowing must
        // never turn into a relation to an unrelated global homonym.
        #[derive(Default)]
        struct Bindings {
            names: BTreeSet<String>,
            imports: bool,
        }
        impl<'a> Visit<'a> for Bindings {
            fn visit_pat_ident(&mut self, p: &'a syn::PatIdent) {
                self.names.insert(p.ident.to_string());
                syn::visit::visit_pat_ident(self, p);
            }
            fn visit_item_use(&mut self, _: &'a syn::ItemUse) {
                self.imports = true;
            }
            fn visit_macro(&mut self, _: &'a syn::Macro) {
                self.imports = true;
            }
            fn visit_block(&mut self, block: &'a syn::Block) {
                for statement in &block.stmts {
                    if let syn::Stmt::Item(item) = statement {
                        match item {
                            Item::Fn(f) => {
                                self.names.insert(f.sig.ident.to_string());
                            }
                            Item::Const(c) => {
                                self.names.insert(c.ident.to_string());
                            }
                            Item::Static(s) => {
                                self.names.insert(s.ident.to_string());
                            }
                            _ => {}
                        }
                    }
                }
                syn::visit::visit_block(self, block);
            }
            fn visit_item_mod(&mut self, _: &'a syn::ItemMod) {}
        }
        struct Calls<'a> {
            positions: Positions<'a>,
            base: usize,
            bindings: Bindings,
            facts: &'a mut Facts,
            receivers: BTreeSet<(String, i64, i64)>,
        }
        impl<'a> Visit<'a> for Calls<'_> {
            fn visit_expr_method_call(&mut self, e: &'a syn::ExprMethodCall) {
                let offset = self.base + e.span().byte_range().start;
                if let Some((line, col)) = self.positions.get(offset) {
                    self.receivers.insert((e.method.to_string(), line, col));
                }
                syn::visit::visit_expr_method_call(self, e);
            }
            fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
                if f.attrs.iter().all(|a| a.path().is_ident("doc")) {
                    syn::visit::visit_item_fn(self, f);
                }
            }
            fn visit_expr_call(&mut self, e: &'a syn::ExprCall) {
                if let syn::Expr::Path(p) = &*e.func {
                    if p.qself.is_none() && p.path.leading_colon.is_none() {
                        let parts: Vec<String> = p
                            .path
                            .segments
                            .iter()
                            .map(|s| s.ident.to_string())
                            .collect();
                        let bare = parts.len() == 1;
                        if !parts.is_empty()
                            && !self.bindings.names.contains(&parts[0])
                            && (!bare
                                || (!self.bindings.imports
                                    && self.facts.functions.contains_key(&parts[0])))
                        {
                            let offset = self.base + p.span().byte_range().start;
                            if let Some((line, col)) = self.positions.get(offset) {
                                self.facts
                                    .calls
                                    .insert((parts.join("::"), line, col), parts);
                            }
                        }
                    }
                }
                syn::visit::visit_expr_call(self, e);
            }
            fn visit_item_mod(&mut self, _: &'a syn::ItemMod) {}
            fn visit_item_impl(&mut self, _: &'a syn::ItemImpl) {}
            fn visit_item_trait(&mut self, _: &'a syn::ItemTrait) {}
        }
        let mut bindings = Bindings::default();
        bindings.visit_file(file);
        let mut calls = Calls {
            positions: Positions::new(source),
            base,
            bindings,
            facts: &mut facts,
            receivers: BTreeSet::new(),
        };
        calls.visit_file(file);
        for receiver in &calls.receivers {
            calls.facts.calls.remove(receiver);
        }
        Ok(facts)
    })
}

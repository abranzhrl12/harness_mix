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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rust_free_calls_position_index_matches_prefix_every_byte() {
        for source in [
            "",
            "\n",
            "\r\n",
            "\u{feff}fn é() { 😀 }\r\n\n終",
            "a\nb\r\nc\n",
        ] {
            let positions = Positions::new(source);
            for offset in 0..=source.len() + 1 {
                let expected = source.get(..offset).map(|prefix| {
                    (
                        prefix.bytes().filter(|b| *b == b'\n').count() as i64 + 1,
                        prefix.rsplit('\n').next().unwrap().encode_utf16().count() as i64,
                    )
                });
                assert_eq!(
                    positions.get(offset),
                    expected,
                    "offset {offset} in {source:?}"
                );
            }
        }
    }
    #[test]
    #[ignore = "focused phase timing; no index reads or writes"]
    fn rust_resolution_phase_probe() {
        use std::{hint::black_box, time::Instant};
        let sources = [
            (
                "src/adapters/index_store.rs",
                include_str!("index_store.rs"),
            ),
            ("src/adapters/process.rs", include_str!("process.rs")),
            ("src/adapters/explore.rs", include_str!("explore.rs")),
            (
                "crates/application/src/lib.rs",
                include_str!("../../crates/application/src/lib.rs"),
            ),
        ];
        let mut extract = 0;
        let mut syntax = 0;
        let mut positions = 0;
        let mut indexed_positions = 0;
        let context = OperationContext::default();
        for _ in 0..10 {
            for (path, source) in sources {
                let started = Instant::now();
                black_box(
                    super::super::super::index_extract::extract(path, source, &context).unwrap(),
                );
                extract += started.elapsed().as_micros();
                let started = Instant::now();
                black_box(analyze(source, &context).unwrap());
                syntax += started.elapsed().as_micros();
                let started = Instant::now();
                for (offset, _) in source.char_indices().step_by(97) {
                    let prefix = &source[..offset];
                    black_box((
                        prefix.bytes().filter(|b| *b == b'\n').count() + 1,
                        prefix.rsplit('\n').next().unwrap().encode_utf16().count(),
                    ));
                }
                positions += started.elapsed().as_micros();
                let started = Instant::now();
                let index = Positions::new(source);
                for (offset, _) in source.char_indices().step_by(97) {
                    black_box(index.get(offset).unwrap());
                }
                indexed_positions += started.elapsed().as_micros();
            }
        }
        println!(
            "{}",
            serde_json::json!({"probe":"rust-resolution-phases","rounds":10,"filesPerRound":4,"debugAssertions":cfg!(debug_assertions),"extractionMs":extract as f64/1000.0,"rustProofMs":syntax as f64/1000.0,"prefixPositionsMs":positions as f64/1000.0,"indexedPositionsMs":indexed_positions as f64/1000.0,"includesDatabase":false})
        );
    }
    #[cfg(windows)]
    #[test]
    fn rust_free_calls_real_sync_workspace_binding() {
        use super::super::{initialize, sync_file};
        let root = tempfile::tempdir().unwrap();
        let context = OperationContext::default();
        initialize(root.path(), &context).unwrap();
        std::fs::create_dir_all(root.path().join("src/adapters")).unwrap();
        for (path, source) in [
            ("src/lib.rs", include_str!("../lib.rs")),
            ("src/adapters/mod.rs", include_str!("mod.rs")),
            (
                "src/adapters/index_store.rs",
                include_str!("index_store.rs"),
            ),
            ("src/adapters/explore.rs", include_str!("explore.rs")),
        ] {
            std::fs::write(root.path().join(path), source).unwrap();
            sync_file(root.path(), path, &context).unwrap();
        }
        let db = rusqlite::Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
        let count: i64 = db.query_row("SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.name='synchronized' AND t.name='sync_workspace_all' AND json_extract(e.metadata,'$.resolvedBy')='syn-module-subset-v1'", [], |r| r.get(0)).unwrap();
        assert_eq!(
            count, 1,
            "the actual harness call must resolve, not just a synthetic fixture"
        );
    }
    #[cfg(windows)]
    #[test]
    fn rust_free_calls_publication_proves_sibling_and_rejects_homonyms() {
        use super::super::{initialize, sync_file};
        let root = tempfile::tempdir().unwrap();
        let context = OperationContext::default();
        initialize(root.path(), &context).unwrap();
        std::fs::create_dir_all(root.path().join("src/adapters")).unwrap();
        let files = [
            ("src/lib.rs", "mod adapters;"),
            ("src/adapters/mod.rs", "mod explore; mod index_store;"),
            (
                "src/adapters/index_store.rs",
                "/// Indexed target\npub(super) fn sync_workspace_all() {}",
            ),
            (
                "src/adapters/explore.rs",
                "fn collect() {}\nfn execute() {}\nfn run() {}\nfn good() {}\nfn synchronized() { x.collect(); stmt.execute(); let run = || {}; run(); good(); super::index_store::sync_workspace_all(); }\n",
            ),
        ];
        for (path, text) in files {
            std::fs::write(root.path().join(path), text).unwrap();
            sync_file(root.path(), path, &context).unwrap();
        }
        let db = rusqlite::Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
        let mut stmt = db.prepare("SELECT t.name FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' ORDER BY t.name").unwrap();
        let targets = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(targets, ["good", "sync_workspace_all"]);
        let pending: i64 = db
            .query_row(
                "SELECT count(*) FROM unresolved_refs WHERE reference_kind='calls'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(pending >= 3);
        drop(stmt);
        let sibling_calls = || {
            db.query_row("SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.name='sync_workspace_all'", [], |r| r.get::<_, i64>(0)).unwrap()
        };
        std::fs::write(
            root.path().join("src/adapters/mod.rs"),
            "mod explore; #[path=\"other.rs\"] mod index_store;",
        )
        .unwrap();
        sync_file(root.path(), "src/adapters/mod.rs", &context).unwrap();
        assert_eq!(
            sibling_calls(),
            0,
            "changed module proof must invalidate the old edge"
        );
        std::fs::write(
            root.path().join("src/adapters/mod.rs"),
            "mod explore; mod index_store;",
        )
        .unwrap();
        sync_file(root.path(), "src/adapters/mod.rs", &context).unwrap();
        assert_eq!(
            sibling_calls(),
            1,
            "pending qualified calls must retry after module changes"
        );
        std::fs::create_dir(root.path().join("src/adapters/index_store")).unwrap();
        std::fs::write(
            root.path().join("src/adapters/index_store/mod.rs"),
            "pub fn sync_workspace_all() {}",
        )
        .unwrap();
        sync_file(root.path(), "src/adapters/index_store/mod.rs", &context).unwrap();
        assert_eq!(sibling_calls(), 0, "ambiguous module files must not bind");
    }
    #[test]
    fn rust_free_calls_reject_methods_and_shadowed_closures() {
        let source = "fn collect() {}\nfn run() {}\nfn good() {}\nfn caller() { x.collect(); let run = || {}; run(); good(); super::store::sync_workspace_all(); }\n";
        let facts = analyze(source, &OperationContext::default()).unwrap();
        let names: Vec<_> = facts.calls.keys().map(|k| k.0.as_str()).collect();
        assert_eq!(names, vec!["good", "super::store::sync_workspace_all"]);
    }
    #[test]
    fn rust_free_calls_reject_parameters_patterns_and_imports() {
        for body in [
            "fn f() { run().run(); }",
            "fn f(run: fn()) { run(); }",
            "fn f() { match x { Some(run) => run(), _ => () } }",
            "use other::run; fn f() { run(); }",
        ] {
            let facts = analyze(
                &format!("fn run() {{}} {body}"),
                &OperationContext::default(),
            )
            .unwrap();
            assert!(facts.calls.is_empty());
        }
    }
    #[test]
    fn rust_free_calls_use_utf16_columns_and_preserve_bom_crlf() {
        let source = "\u{feff}fn good() {}\r\nfn caller() { let s = \"😀\"; good(); }\r\n";
        let facts = analyze(source, &OperationContext::default()).unwrap();
        let offset = source.rfind("good()").unwrap();
        let prefix = &source[..offset];
        let col = prefix.rsplit('\n').next().unwrap().encode_utf16().count() as i64;
        assert!(facts.call("good", 2, col).is_some());
    }
}

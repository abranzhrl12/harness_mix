use harness_domain::execution::OperationContext;
#[path = "../src/call_candidate.rs"]
mod call_candidate;
use call_candidate::Facts as RaFacts;
use ra_syntax_compare::adapters::store;
use std::collections::{BTreeMap, BTreeSet};
use syn::{spanned::Spanned, visit::Visit};

type Probe = (String, i64, i64);

fn position(source: &str, offset: usize) -> (i64, i64) {
    let prefix = &source[..offset];
    (prefix.bytes().filter(|b| *b == b'\n').count() as i64 + 1,
        prefix.rsplit('\n').next().unwrap().encode_utf16().count() as i64)
}

fn prepared(source: &str) -> (String, usize) {
    let base = if source.starts_with('\u{feff}') { 3 } else { 0 };
    let input = &source[base..];
    let normalized = if input.starts_with("#!") && !input.starts_with("#![") {
        let end = input.find('\n').unwrap_or(input.len());
        format!("{}{}", " ".repeat(end), &input[end..])
    } else { input.to_owned() };
    (normalized, base)
}

struct Sites<'a> { source: &'a str, base: usize, probes: BTreeSet<Probe> }
impl<'ast> Visit<'ast> for Sites<'_> {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*call.func {
            if path.qself.is_none() && path.path.leading_colon.is_none() {
                let parts = path.path.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>();
                if !parts.is_empty() {
                    let offset = path.span().byte_range().start + self.base;
                    let (line, col) = position(self.source, offset);
                    self.probes.insert((parts.join("::"), line, col));
                }
            }
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let offset = call.span().byte_range().start + self.base;
        let (line, col) = position(self.source, offset);
        self.probes.insert((call.method.to_string(), line, col));
        syn::visit::visit_expr_method_call(self, call);
    }
}

fn probes(source: &str, ra: &RaFacts) -> Vec<Probe> {
    let (normalized, base) = prepared(source);
    let syn = syn::parse_file(&normalized).unwrap();
    let mut sites = Sites { source, base, probes: BTreeSet::new() };
    sites.visit_file(&syn);
    for (name, line, col, _) in ra.call_entries() {
        sites.probes.insert((name.to_owned(), line, col));
    }
    sites.probes.into_iter().collect()
}

fn assert_matches_production(source: &str) -> (RaFacts, BTreeMap<Probe, Option<Vec<String>>>) {
    let context = OperationContext::default();
    let ra = call_candidate::analyze(source, &context).unwrap();
    let _reserved = ra.reserved();
    let probes = probes(source, &ra);
    let production = store::snapshot(source, &probes, &context).unwrap();
    assert_eq!(production.functions, ra.functions.iter().map(|(n,v)| (n.clone(), *v)).collect::<Vec<_>>(), "functions: {source}");
    assert_eq!(production.modules, ra.modules.iter().cloned().collect::<Vec<_>>(), "modules: {source}");
    let ra_queries = probes.iter().cloned().map(|(name,line,col)| {
        let value = ra.call(&name,line,col).map(<[String]>::to_vec);
        ((name,line,col),value)
    }).collect::<BTreeMap<_,_>>();
    assert_eq!(production.call_queries, ra_queries, "call lookups: {source}");
    (ra, ra_queries)
}

fn at(source: &str, needle: &str, occurrence: usize) -> (i64, i64) {
    let offset = source.match_indices(needle).nth(occurrence).unwrap().0;
    position(source, offset)
}

#[test]
fn positive_bare_and_sibling_qualified_calls_match_production() {
    let bare = "pub fn target() {}\nfn caller() { target(); }\n";
    let (facts, _) = assert_matches_production(bare);
    let (line,col) = at(bare, "target();", 0);
    assert_eq!(facts.call("target",line,col), Some(["target".to_owned()].as_slice()));

    let sibling = "pub fn target() {}\nmod sibling { pub fn target() {} }\nfn caller() { sibling::target(); target(); }\n";
    let (facts, _) = assert_matches_production(sibling);
    let (line,col) = at(sibling, "sibling::target", 0);
    assert_eq!(facts.call("sibling::target",line,col), Some(["sibling".to_owned(),"target".to_owned()].as_slice()));
    let (line,col) = at(sibling, "target();", 1);
    assert_eq!(facts.call("target",line,col), Some(["target".to_owned()].as_slice()));
}

#[test]
fn parameters_closures_patterns_imports_and_macros_block_bare_calls() {
    let fixtures = [
        "fn run() {}\nfn caller(run: fn()) { run(); }\n",
        "fn run() {}\nfn caller() { let run = || {}; run(); }\n",
        "fn run() {}\nfn caller() { if let Some(run) = value { run(); } }\n",
        "fn run() {}\nstruct S { run: fn() }\nfn caller(v: S) { let S { run } = v; run(); }\n",
        "use crate::other;\nfn run() {}\nfn caller() { run(); }\n",
        "macro_rules! any { () => {} }\nfn run() {}\nfn caller() { run(); }\n",
        "fn run() {}\nfn caller() { { use crate::other; } run(); }\n",
    ];
    for source in fixtures {
        let (_, queries) = assert_matches_production(source);
        assert!(queries.iter().filter(|((name,_,_),_)| name == "run")
            .all(|(_,value)| value.is_none()), "unexpected bare call in {source}");
    }
}

#[test]
fn methods_free_calls_nested_functions_and_scoped_items_keep_exact_policy() {
    let source = "fn reset() {}\nfn caller() { obj.reset(); reset(); }\n";
    let (facts, _) = assert_matches_production(source);
    let (line,col) = at(source, "reset();", 1);
    assert_eq!(facts.call("reset",line,col), Some(["reset".to_owned()].as_slice()));
    let (line,col) = at(source, "obj.reset()", 0);
    assert_eq!(facts.call("reset",line,col), None);

    let nested = "fn target() {}\nfn caller() { fn inner() { target(); } inner(); }\n";
    let (facts,_) = assert_matches_production(nested);
    let (line,col) = at(nested,"target();",0);
    assert_eq!(facts.call("target",line,col),Some(["target".to_owned()].as_slice()));
    let (line,col) = at(nested,"inner();",0);
    assert_eq!(facts.call("inner",line,col),None);

    let inner_module = "mod inner { use crate::other; fn hidden() {} }\nfn target() {}\nfn caller() { target(); }\n";
    let (facts,_) = assert_matches_production(inner_module);
    let (line,col) = at(inner_module,"target();",0);
    assert_eq!(facts.call("target",line,col),Some(["target".to_owned()].as_slice()));
}

#[test]
fn doc_prefix_is_exact_and_spaced_restricted_visibility_matches_syn() {
    let source = "fn target() {}\n#[docx = \"not a doc attribute\"] fn decorated() { target(); }\npub ( crate ) fn scoped() {}\nfn caller() { scoped(); }\n";
    let (facts, queries) = assert_matches_production(source);
    assert!(!facts.functions.contains_key("decorated"));
    assert_eq!(facts.functions.get("scoped"), Some(&(3, true)));
    let (line,col) = at(source, "target();", 0);
    assert!(queries.get(&("target".to_owned(),line,col)).unwrap().is_none());
    let (line,col) = at(source, "scoped();", 0);
    assert_eq!(facts.call("scoped",line,col),Some(["scoped".to_owned()].as_slice()));
}

#[test]
fn qualified_self_and_leading_colon_calls_are_rejected() {
    let source = "trait Trait { fn target(); }\nstruct S;\nimpl Trait for S { fn target() {} }\nfn caller<T: Trait>() { ::external::target(); <T as Trait>::target(); }\n";
    let (facts,queries) = assert_matches_production(source);
    assert!(queries.iter().filter(|((name,_,_),_)| name.contains("target"))
        .all(|(_,value)| value.is_none()), "unexpected qualified-self/leading-colon resolution: {queries:?}");
    let (line,col) = at(source,"::external::target",0);
    assert_eq!(facts.call("external::target",line,col),None);
}

#[test]
fn cfg_module_facts_and_utf16_bom_shebang_crlf_positions_match() {
    let modules = "mod api;\n/// docs exclude this from the cfg-only module rule\nmod documented;\n#[cfg(windows)] mod windows;\n#[cfg(windows)] #[cfg(windows)] mod redundant;\n#[cfg(windows)] #[cfg(unix)] mod mixed;\n#[cfg(unix)] mod unix;\n#[cfg(any(windows))] mod broad;\n";
    let (facts,_) = assert_matches_production(modules);
    assert!(facts.modules.contains("api"));
    assert!(!facts.modules.contains("documented"));
    assert_eq!(facts.modules.contains("redundant"), cfg!(windows));
    assert!(!facts.modules.contains("mixed"));
    assert_eq!(facts.modules.contains("windows"), cfg!(windows));
    assert!(!facts.modules.contains("unix"));
    assert!(!facts.modules.contains("broad"));

    let source = "\u{feff}#!/usr/bin/env rustx\r\n/// docs\r\npub fn target() {}\r\nfn caller() { let emoji = \"😀\"; target(); }\r\n";
    let (facts,_) = assert_matches_production(source);
    let offset = source.rfind("target();").unwrap();
    let (line,col) = position(source,offset);
    assert_eq!((line,col),(4,32));
    assert_eq!(facts.call("target",line,col),Some(["target".to_owned()].as_slice()));
}

#[test]
fn production_baseline_and_candidate_both_reject_invalid_input() {
    for source in ["fn bad( {", "fn good() {}\nfn unfinished("] {
        let context = OperationContext::default();
        assert!(store::snapshot(source,&[],&context).is_err());
        assert!(call_candidate::analyze(source,&context).is_err());
    }
}

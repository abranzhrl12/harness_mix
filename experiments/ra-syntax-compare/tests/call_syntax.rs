//! Syntax/coordinate parity only: this does NOT certify graph binding or resolution.
use ra_ap_syntax::{ast, AstNode, Edition, SourceFile};
use syn::{spanned::Spanned, visit::Visit};

#[test]
fn call_forms_preserve_utf16_expression_ranges() {
    let source = "fn good() {}\r\nfn f(run: fn()) { let s=\"😀\"; good(); run(); x.good(); super::store::save(); (good)(); }";
    type Site = (String, usize, usize);
    struct Calls<'a> { source: &'a str, sites: Vec<Site> }
    impl<'ast> Visit<'ast> for Calls<'_> {
        fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
            let span = call.span().byte_range();
            self.sites.push(("call".into(), self.source[..span.start].encode_utf16().count(),
                self.source[..span.end].encode_utf16().count()));
            syn::visit::visit_expr_call(self, call);
        }
        fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
            let span = call.span().byte_range();
            self.sites.push(("method".into(), self.source[..span.start].encode_utf16().count(),
                self.source[..span.end].encode_utf16().count()));
            syn::visit::visit_expr_method_call(self, call);
        }
    }
    let mut syn_sites = {
        let tree = syn::parse_file(source).unwrap();
        let mut visitor = Calls { source, sites: Vec::new() };
        visitor.visit_file(&tree);
        visitor.sites
    };
    proc_macro2::extra::invalidate_current_thread_spans();
    let parsed = SourceFile::parse(source, Edition::Edition2024);
    assert!(parsed.errors().is_empty());
    let mut ra_sites: Vec<Site> = parsed.syntax_node().descendants().filter_map(|node| {
        let kind = if ast::CallExpr::can_cast(node.kind()) { "call" }
            else if ast::MethodCallExpr::can_cast(node.kind()) { "method" }
            else { return None };
        let range = node.text_range();
        Some((kind.to_owned(), source[..usize::from(range.start())].encode_utf16().count(),
            source[..usize::from(range.end())].encode_utf16().count()))
    }).collect();
    syn_sites.sort();
    ra_sites.sort();
    assert_eq!(syn_sites, ra_sites);
    assert_eq!(syn_sites.len(), 5);
}

    use super::*;
    use serde_json::json;

    #[test]
    fn local_explicit_trait_impl_adds_typed_edge_and_is_idempotent() {
        let source = "trait Render { fn draw(&self); } struct Card; impl Render for Card { fn draw(&self) {} }";
        let trait_method = node("trait-method", "draw", "function", 1, 15);
        let impl_method = node("impl-method", "draw", "method", 1, 69);
        let mut graph = json!({
            "nodes": [trait_method, impl_method],
            "edges": [],
            "unresolvedReferences": []
        });
        enrich(
            "src/lib.rs",
            source,
            &mut graph,
            &OperationContext::default(),
        )
        .unwrap();
        assert_eq!(graph["edges"][0]["kind"], "implements");
        assert_eq!(graph["edges"][0]["source"], "trait-method");
        assert_eq!(graph["edges"][0]["target"], "impl-method");
        assert_eq!(
            graph["edges"][0]["metadata"]["resolvedBy"],
            "rust-local-trait-impl-v1"
        );
        assert_eq!(
            graph["edges"][0]["metadata"]["relationSemantics"],
            "implementation-not-invocation"
        );
        enrich(
            "src/lib.rs",
            source,
            &mut graph,
            &OperationContext::default(),
        )
        .unwrap();
        assert_eq!(graph["edges"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn rejects_aliases_generics_cfg_negative_and_ambiguous_traits() {
        for source in [
            "trait Render { fn draw(&self); } struct Card; impl other::Render for Card { fn draw(&self) {} }",
            "trait Render<T> { fn draw(&self); } struct Card; impl Render<u8> for Card { fn draw(&self) {} }",
            "#[cfg(test)] trait Render { fn draw(&self); } struct Card; impl Render for Card { fn draw(&self) {} }",
            "trait Render { #[cfg(test)] fn draw(&self); } struct Card; impl Render for Card { fn draw(&self) {} }",
            "trait Render { fn draw(&self); } struct Card; impl Render for Card { #[cfg(test)] fn draw(&self) {} }",
            "trait Render { fn draw(&self); } struct Card; impl !Render for Card { fn draw(&self) {} }",
            "trait Render { fn draw(&self); } trait Render { fn draw(&self); } struct Card; impl Render for Card { fn draw(&self) {} }",
        ] {
            let mut graph = json!({"nodes":[],"edges":[],"unresolvedReferences":[]});
            enrich(
                "src/lib.rs",
                source,
                &mut graph,
                &OperationContext::default(),
            )
            .unwrap();
            assert!(graph["edges"].as_array().unwrap().is_empty(), "{source}");
        }
    }

    #[test]
    fn cfg_method_is_rejected_even_with_matching_graph_endpoints() {
        let source = "trait Render { #[cfg(test)] fn draw(&self); } struct Card; impl Render for Card { #[cfg(test)] fn draw(&self) {} }";
        let trait_start = source.find("#[cfg(test)]").unwrap();
        let impl_start = source.rfind("#[cfg(test)]").unwrap();
        let mut graph = json!({
            "nodes": [
                node_at(source, "trait-method", "draw", "function", trait_start),
                node_at(source, "impl-method", "draw", "method", impl_start)
            ],
            "edges": [],
            "unresolvedReferences": []
        });
        enrich(
            "src/lib.rs",
            source,
            &mut graph,
            &OperationContext::default(),
        )
        .unwrap();
        assert!(graph["edges"].as_array().unwrap().is_empty());
    }

    #[test]
    fn file_cfg_is_not_reported_as_an_unconditional_implementation() {
        let source = "#![cfg(test)]\ntrait Render { fn draw(&self); } struct Card; impl Render for Card { fn draw(&self) {} }";
        let mut graph = json!({"nodes":[
            node_at(source,"trait-method","draw","function",source.find("fn draw").unwrap()),
            node_at(source,"impl-method","draw","method",source.rfind("fn draw").unwrap())
        ],"edges":[]});
        enrich(
            "src/lib.rs",
            source,
            &mut graph,
            &OperationContext::default(),
        )
        .unwrap();
        assert!(graph["edges"].as_array().unwrap().is_empty());
    }

    fn node(id: &str, name: &str, kind: &str, line: u64, column: u64) -> Value {
        json!({
            "id": id,
            "name": name,
            "kind": kind,
            "filePath": "src/lib.rs",
            "language": "rust",
            "startLine": line,
            "startColumn": column
        })
    }

    fn node_at(source: &str, id: &str, name: &str, kind: &str, offset: usize) -> Value {
        let prefix = &source[..offset];
        let line = prefix.bytes().filter(|byte| *byte == b'\n').count() as u64 + 1;
        let column = prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u64;
        node(id, name, kind, line, column)
    }

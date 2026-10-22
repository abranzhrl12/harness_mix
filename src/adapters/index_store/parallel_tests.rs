use super::*;

#[test]
fn changed_source_never_reuses_a_prepared_graph() {
    let prepared = Prepared {
        digest: hash(b"old"),
        graph: Ok(json!({})),
        diagnostic: None,
    };
    assert!(prepared.consume(b"new").is_none());
}

#[test]
fn parallel_preparation_preserves_order_graphs_and_parser_errors() {
    let dir = tempfile::tempdir().unwrap();
    let paths = vec!["a.rs".to_owned(), "b.rs".to_owned()];
    let sources = ["pub fn valid() {}", "pub fn broken( {"];
    for (path, source) in paths.iter().zip(sources) {
        std::fs::write(dir.path().join(path), source).unwrap();
    }
    let context = OperationContext::default();
    let batch = prepare(dir.path(), &paths.iter().collect::<Vec<_>>(), &context);
    for ((prepared, path), source) in batch.into_iter().zip(&paths).zip(sources) {
        let parallel = prepared.unwrap().consume(source.as_bytes()).unwrap();
        let parallel_diagnostic = index_extract::take_diagnostic();
        let sequential = index_extract::extract(path, source, &context);
        assert_eq!(parallel, sequential);
        assert_eq!(parallel_diagnostic, index_extract::take_diagnostic());
    }
}

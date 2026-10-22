//! Bounded source preparation. Workers never open the graph database.
use super::*;

pub(super) struct Prepared {
    digest: String,
    graph: Result<Value, Failure>,
    diagnostic: Option<Value>,
}

impl Prepared {
    pub(super) fn consume(self, bytes: &[u8]) -> Option<Result<Value, Failure>> {
        if self.digest != hash(bytes) {
            return None;
        }
        if let Some(diagnostic) = self.diagnostic {
            index_extract::set_diagnostic(diagnostic);
        }
        Some(self.graph)
    }
}

pub(super) fn workers() -> usize {
    let automatic = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(4);
    std::env::var("HARNESS_INDEX_WORKERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(automatic)
        .clamp(1, 4)
}

pub(super) fn prepare(
    root: &Path,
    paths: &[&String],
    context: &OperationContext,
) -> Vec<Option<Prepared>> {
    // Each wave holds at most four source graphs, independent of workspace size.
    // Fall back to the ordinary file path for I/O/scope failures; it owns reporting.
    std::thread::scope(|threads| {
        let handles: Vec<_> = paths.iter().map(|path| {
            threads.spawn(move || {
                let context = context.with_execution_timeout(Some(Duration::from_secs(60)));
                let project = Project::new(root).ok()?;
                let scope = scope::Snapshot::load(&project, &context).ok()?;
                if scope.excluded(path) { return None; }
                let bytes = read_source(&project, path, &context).ok()??;
                let source = std::str::from_utf8(&bytes).ok()?;
                let graph = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    index_extract::extract(path, source, &context)
                })).unwrap_or_else(|panic| {
                    let message = panic.downcast_ref::<String>().map(String::as_str)
                        .or_else(|| panic.downcast_ref::<&str>().copied())
                        .unwrap_or("Extractor panicked without a text payload");
                    index_extract::set_diagnostic(json!({"file":path,"stage":"extract_source","message":message,"panic":true}));
                    Err(("EXTRACTION_PANIC", "Source extractor panicked; file was not published"))
                });
                Some(Prepared {
                    digest: hash(&bytes),
                    graph,
                    diagnostic: index_extract::take_diagnostic(),
                })
            })
        }).collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap_or(None))
            .collect()
    })
}

#[cfg(test)]
mod tests {
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
}

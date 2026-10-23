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
                let source = harness_domain::encoding::decode_source(&bytes).ok()?;
                let graph = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    index_extract::extract(path, source.as_ref(), &context)
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
#[path = "parallel_tests.rs"]
mod tests;

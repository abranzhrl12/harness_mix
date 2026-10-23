pub(in crate::adapters) use harness_domain::file_freshness::Verdict;

/// A symbol candidate read from the graph, including its indexed content hash.
#[derive(Clone, Debug)]
pub(in crate::adapters) struct Candidate {
    pub(in crate::adapters) id: String,
    pub(in crate::adapters) name: String,
    pub(in crate::adapters) kind: String,
    pub(in crate::adapters) qualified_name: String,
    pub(in crate::adapters) file_path: String,
    pub(in crate::adapters) start_line: u32,
    pub(in crate::adapters) end_line: u32,
    pub(in crate::adapters) signature: String,
    pub(in crate::adapters) docstring: String,
    pub(in crate::adapters) language: String,
    pub(in crate::adapters) indexed_hash: Option<String>,
}

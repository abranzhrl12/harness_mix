/// A symbol candidate read from the graph, with the stamp that certifies it.
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

/// Freshness of one indexed file relative to the working tree.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::adapters) enum Verdict {
    /// The indexed content hash still matches the file on disk.
    Fresh,
    /// The file changed after it was indexed.
    Stale,
    /// The indexed file no longer exists.
    Missing,
    /// Freshness could not be established.
    Unknown,
}

impl Verdict {
    pub(in crate::adapters) fn as_str(self) -> &'static str {
        match self {
            Verdict::Fresh => "fresh",
            Verdict::Stale => "stale",
            Verdict::Missing => "missing",
            Verdict::Unknown => "unknown",
        }
    }
}

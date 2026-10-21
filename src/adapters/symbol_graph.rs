//! Graph-backed symbol discovery certified against the working tree.
//!
//! The index is a cache, never a second source of truth: candidates come from
//! the graph, every candidate file is certified against the working tree, and
//! files that are not fresh are reparsed live before their rows are published.
//!
//! Three phases, in this order:
//!   1. discover: candidate symbols from `nodes` (bounded SQL, no file reads)
//!   2. certify:  compare the indexed content hash with the file on disk
//!   3. repair:   reparse stale files live so published rows stay true
mod cache;
mod classification;
mod discovery;
mod freshness;
mod model;

pub(super) use classification::graph_kinds;
pub(in crate::adapters) use classification::{SELECTOR_KINDS, canonical_kind};
pub(in crate::adapters) use discovery::discover;
pub(in crate::adapters) use freshness::{certify, freshness_summary};
pub(in crate::adapters) use model::{Candidate, Verdict};

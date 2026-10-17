//! Transport-independent contracts, hashes and result policy. No filesystem or MCP SDK.
pub mod contracts;
pub mod editing;
pub mod execution;
pub mod freshness;
pub mod log_archive;
pub mod navigation;
pub mod process;
pub mod process_output;
pub mod results;
pub mod sql;
pub mod workflow;
pub mod xlsx;
pub mod svg;

pub type Failure = (&'static str, &'static str);

pub fn hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

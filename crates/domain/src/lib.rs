//! Transport-independent contracts, hashes and result policy. No filesystem or MCP SDK.
pub mod assistance;
pub mod contracts;
pub mod diagnostics;
pub mod editing;
pub mod execution;
pub mod freshness;
pub mod handover;
pub mod jobs;
pub mod log_archive;
pub mod navigation;
pub mod parsing;
pub mod process;
pub mod process_output;
pub mod results;
pub mod sql;
pub mod svg;
pub mod tasks;
pub mod tool_contract;
pub mod workflow;
pub mod xlsx;

pub type Failure = (&'static str, &'static str);

pub fn hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

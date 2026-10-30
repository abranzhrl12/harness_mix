//! Transport-independent contracts, hashes and result policy. No filesystem or MCP SDK.
pub mod assistance;
pub mod browser;
pub mod contracts;
pub mod diagnostics;
pub mod editing;
pub mod encoding;
pub mod execution;
pub mod file_freshness;
pub mod freshness;
pub mod handover;
pub mod hashing;
pub mod indexing;
pub mod jobs;
pub mod log_archive;
pub mod lsp;
pub mod navigation;
pub mod parsing;
pub mod process;
pub mod process_output;
pub mod resolution_policy;
pub mod results;
pub mod sql;
pub mod svg;
pub mod tasks;
pub mod tool_contract;
pub mod workflow;
pub mod word;
pub mod xlsx;

pub type Failure = (&'static str, &'static str);

pub fn hash(bytes: &[u8]) -> String {
    hashing::sha256_digest(bytes).to_hex()
}

#[cfg(test)]
mod hash_tests {
    use super::{hash, hashing::sha256_digest};
    use sha2::{Digest, Sha256};

    #[test]
    fn compatibility_hash_remains_lowercase_sha256_hex() {
        let digest = sha256_digest(b"abc");
        assert_eq!(
            digest.to_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(hash(b"abc"), digest.to_hex());
    }

    #[test]
    fn compatibility_hash_matches_the_previous_formatter_for_arbitrary_bytes() {
        for bytes in [
            Vec::new(),
            vec![0xff],
            vec![0, 0xff, 0x80, b'a'],
            (0..31).collect(),
            (0..32).collect(),
            (0..64).collect(),
            vec![b'x'; 1024],
        ] {
            assert_eq!(hash(&bytes), format!("{:x}", Sha256::digest(&bytes)));
        }
    }
}

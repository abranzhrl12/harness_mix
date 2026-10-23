//! SHA-256 digest representation and the current hashing provider.

use sha2::{Digest, Sha256, digest::Output};
use std::fmt;

/// A validated SHA-256 digest, independent of its textual representation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Sha256Digest(Output<Sha256>);

impl Sha256Digest {
    /// Parse exactly 64 ASCII hexadecimal characters, accepting either case.
    pub fn parse(value: &str) -> Option<Self> {
        let bytes = value.as_bytes();
        if bytes.len() != 64 {
            return None;
        }

        let mut digest = Output::<Sha256>::default();
        for (index, pair) in bytes.chunks_exact(2).enumerate() {
            digest[index] = (hex_value(pair[0])? << 4) | hex_value(pair[1])?;
        }
        Some(Self(digest))
    }

    /// Return the canonical lowercase hexadecimal form used by existing contracts.
    pub fn to_hex(self) -> String {
        format!("{:x}", self.0)
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:x}", self.0)
    }
}

/// Calculate the current SHA-256 digest for exact input bytes.
pub fn sha256_digest(bytes: &[u8]) -> Sha256Digest {
    Sha256Digest(Sha256::digest(bytes))
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Sha256Digest, sha256_digest};

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            sha256_digest(b"").to_hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_digest(b"abc").to_hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn parsing_accepts_uppercase_and_encodes_canonically() {
        let lowercase = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let uppercase = lowercase.to_ascii_uppercase();
        let parsed_lower = Sha256Digest::parse(lowercase).unwrap();
        let parsed_upper = Sha256Digest::parse(&uppercase).unwrap();

        assert_eq!(parsed_lower, parsed_upper);
        assert_eq!(parsed_upper.to_hex(), lowercase);
        assert_eq!(parsed_upper.to_string(), lowercase);
        assert!(Sha256Digest::parse(&lowercase[..63]).is_none());
        assert!(Sha256Digest::parse(&format!("{lowercase}0")).is_none());
        assert!(Sha256Digest::parse(&format!("{}g", &lowercase[..63])).is_none());
    }
}

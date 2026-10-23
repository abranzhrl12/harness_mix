use crate::hashing::Sha256Digest;

/// Freshness of an indexed file relative to the current working tree.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// The indexed content hash still matches the current file content.
    Fresh,
    /// The current file content differs from the indexed hash.
    Stale,
    /// The indexed file is no longer present in the working tree.
    Missing,
    /// Freshness could not be established.
    Unknown,
}

impl Verdict {
    /// Classify current file content against its indexed hash.
    ///
    /// The comparison is exact so uppercase or malformed indexed hashes remain
    /// stale, matching the existing index contract.
    pub fn from_indexed_hash(indexed_hash: Option<&str>, current: &Sha256Digest) -> Self {
        match indexed_hash {
            Some(indexed) if indexed == current.to_hex() => Self::Fresh,
            Some(_) => Self::Stale,
            None => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Missing => "missing",
            Self::Unknown => "unknown",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Verdict;
    use crate::hashing::sha256_digest;

    #[test]
    fn hash_classification_preserves_exact_index_text_semantics() {
        let current = sha256_digest(b"source");
        let hash = current.to_hex();

        assert_eq!(Verdict::from_indexed_hash(Some(&hash), &current), Verdict::Fresh);
        assert_eq!(
            Verdict::from_indexed_hash(Some(&hash.to_uppercase()), &current),
            Verdict::Stale
        );
        assert_eq!(
            Verdict::from_indexed_hash(Some("different"), &current),
            Verdict::Stale
        );
        assert_eq!(Verdict::from_indexed_hash(None, &current), Verdict::Unknown);
    }
}

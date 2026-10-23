//! Pure source observations and incremental classification.
pub use harness_domain::indexing::{Observation, Plan};
use harness_domain::{
    Failure,
    execution::OperationContext,
    hashing::{Sha256Digest, sha256_digest},
};
use std::collections::BTreeMap;

/// Coordinates one indexed snapshot, source inventory, classification and optional
/// preview. The adapter must hold the same read transaction throughout this call.
pub fn plan_workspace<P: crate::ports::indexing::PlanningPort>(
    port: &mut P,
    preview: bool,
    context: &OperationContext,
) -> Result<(Plan, super::inventory::Inventory, serde_json::Value), Failure> {
    context.check()?;
    let indexed = port.indexed(context)?;
    context.check()?;
    let tracked = indexed.keys().cloned().collect::<Vec<_>>();
    let inventory = port.inventory(&tracked, context)?;
    context.check()?;
    context.report_progress("planning", 0, None, None);
    let mut result = classify(&indexed, &inventory.observations, false, context)?;
    let excluded: std::collections::BTreeSet<_> = inventory.scope_excluded.iter().collect();
    result.unchecked.retain(|path| !excluded.contains(path));
    result
        .removed
        .extend(inventory.scope_excluded.iter().cloned());
    result.removed.sort();
    result.removed.dedup();
    result.unchecked.retain(|path| !port.restricted(path));
    let extraction = if preview {
        port.preview(&result, &inventory, context)?
    } else {
        serde_json::Value::Null
    };
    context.check()?;
    Ok((result, inventory, extraction))
}

pub const MAX_FILES: usize = 20_000;
pub const MAX_SOURCE_BYTES: usize = 1024 * 1024;

/// Validate that source bytes can be decoded, but hash the original bytes so a
/// change in encoding or exact source contents always invalidates the snapshot.
/// Oversize markers must NOT be passed as Content: they don't prove freshness.
pub fn source_hash(bytes: &[u8]) -> Result<String, Failure> {
    source_digest_with(bytes, sha256_digest).map(Sha256Digest::to_hex)
}

fn source_digest_with(
    bytes: &[u8],
    calculate: impl FnOnce(&[u8]) -> Sha256Digest,
) -> Result<Sha256Digest, Failure> {
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err((
            "SYNC_LIMIT",
            "Source exceeds the CodeGraph extraction limit",
        ));
    }
    harness_domain::encoding::decode_source(bytes)
        .map_err(|_| ("SYNC_UNCHECKED", "Source cannot be decoded as UTF-8 or Windows-1252"))?;
    Ok(calculate(bytes))
}

pub fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1000
        && !path.contains(['\\', ':'])
        && !path.chars().any(char::is_control)
        && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
}

pub fn valid_hash(value: &str) -> bool {
    Sha256Digest::parse(value).is_some()
}

/// Classify a validated indexed snapshot against conservative source observations.
/// Missing entries imply deletion only when the caller certifies complete coverage.
pub fn classify(
    indexed: &BTreeMap<String, (String, bool)>,
    observed: &BTreeMap<String, Observation>,
    complete: bool,
    context: &OperationContext,
) -> Result<Plan, Failure> {
    context.check()?;
    if observed.len() > MAX_FILES {
        return Err(("SYNC_LIMIT", "Source inventory exceeds the file budget"));
    }
    for (path, observation) in observed {
        context.check()?;
        if !valid_path(path) {
            return Err((
                "SYNC_CONTRACT",
                "Inventory contains an invalid relative path",
            ));
        }
        if let Observation::Content(digest) = observation {
            if Sha256Digest::parse(digest).is_none() {
                return Err(("SYNC_CONTRACT", "Inventory contains an invalid source hash"));
            }
        }
    }
    let mut result = Plan {
        inventory_complete: complete,
        ..Plan::default()
    };
    for (path, (digest, clean)) in indexed {
        context.check()?;
        match observed.get(path) {
            Some(Observation::Content(current)) if !current.eq_ignore_ascii_case(digest) => {
                result.modified.push(path.clone())
            }
            Some(Observation::Content(_)) if !clean => result.retry_extraction.push(path.clone()),
            Some(Observation::Content(_)) => result.unchanged.push(path.clone()),
            Some(Observation::Missing) => result.removed.push(path.clone()),
            None if complete => result.removed.push(path.clone()),
            _ => result.unchecked.push(path.clone()),
        }
    }
    for (path, observation) in observed {
        context.check()?;
        if !indexed.contains_key(path) {
            match observation {
                Observation::Content(_) => result.added.push(path.clone()),
                Observation::Unchecked => result.unchecked.push(path.clone()),
                Observation::Missing => {}
            }
        }
    }
    result.unchecked.sort();
    // Even an empty plan cannot certify extraction, relationships or publication.
    context.check()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::{MAX_SOURCE_BYTES, source_digest_with};
    use harness_domain::{hashing::Sha256Digest, hashing::sha256_digest};
    use std::cell::Cell;

    #[test]
    fn source_validation_is_independent_of_hash_calculation() {
        let called = Cell::new(false);
        let digest =
            Sha256Digest::parse("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
                .unwrap();
        let result = source_digest_with(b"abc", |bytes| {
            called.set(true);
            assert_eq!(bytes, b"abc");
            digest
        })
        .unwrap();
        assert!(called.get());
        assert_eq!(result, digest);

        called.set(false);
        assert_eq!(
            source_digest_with(&[0x81], |_| {
                called.set(true);
                digest
            })
            .unwrap_err()
            .0,
            "SYNC_UNCHECKED"
        );
        assert!(!called.get());

        assert_eq!(
            source_digest_with(&vec![b'x'; MAX_SOURCE_BYTES + 1], |_| {
                called.set(true);
                digest
            })
            .unwrap_err()
            .0,
            "SYNC_LIMIT"
        );
        assert!(!called.get());
        assert_eq!(
            super::source_hash(b"abc").unwrap(),
            sha256_digest(b"abc").to_hex()
        );
        let legacy = b"acci\xf3n";
        assert_eq!(super::source_hash(legacy).unwrap(), sha256_digest(legacy).to_hex());
    }
}

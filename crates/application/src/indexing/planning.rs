//! Pure source observations and incremental classification.
pub use harness_domain::indexing::{Observation, Plan};
use harness_domain::{Failure, execution::OperationContext, hash};
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

/// CodeGraph hashes decoded UTF-8 text, not arbitrary raw bytes. Reject invalid
/// UTF-8 rather than silently introducing a different replacement policy.
/// Oversize markers must NOT be passed as Content: they don't prove freshness.
pub fn source_hash(bytes: &[u8]) -> Result<String, Failure> {
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err((
            "SYNC_LIMIT",
            "Source exceeds the CodeGraph extraction limit",
        ));
    }
    std::str::from_utf8(bytes).map_err(|_| ("SYNC_UNCHECKED", "Source is not UTF-8"))?;
    Ok(hash(bytes))
}

pub fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1000
        && !path.contains(['\\', ':'])
        && !path.chars().any(char::is_control)
        && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
}

pub fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
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
        if let Observation::Content(digest) = observation
            && !valid_hash(digest)
        {
            return Err(("SYNC_CONTRACT", "Inventory contains an invalid source hash"));
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

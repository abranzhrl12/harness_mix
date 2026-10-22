//! Conservative bounded inventory policy. Discovery and source I/O are ports.
use super::planning::{MAX_FILES, MAX_SOURCE_BYTES, Observation, source_hash};
use crate::ports::indexing::{InventoryPort, SourceMetadata};
pub use harness_domain::indexing::Inventory;
use harness_domain::{Failure, execution::OperationContext};
use std::collections::BTreeMap;

pub const MAX_VISITED: usize = 40_000;
pub const MAX_BYTES: u64 = 64 * 1024 * 1024;

pub fn scan<P: InventoryPort>(
    port: &P,
    tracked: &[String],
    context: &OperationContext,
) -> Result<Inventory, Failure> {
    context.check()?;
    context.report_progress("inventory", 0, None, None);
    let mut inventory = Inventory {
        observations: BTreeMap::new(),
        discovery_complete: true,
        visited_entries: 0,
        reserved_source_bytes: 0,
        skipped_entries: 0,
        restricted_indexed_files: 0,
        restricted_indexed_reasons: BTreeMap::new(),
        budget_exhausted: false,
        scope_excluded: Vec::new(),
        scope: port.scope(),
        file_diagnostics: Vec::new(),
    };
    // Check each tracked path directly. Discovery exclusions must never turn an
    // old provider record into an inferred deletion. This is NOT full ScopeIgnore.
    for path in tracked {
        context.check()?;
        if let Some(reason) = port.restricted_reason(path) {
            inventory.restricted_indexed_files += 1;
            *inventory
                .restricted_indexed_reasons
                .entry(reason)
                .or_default() += 1;
            continue;
        }
        if port.excluded(path) {
            inventory.scope_excluded.push(path.clone());
            continue;
        }
        let observation = observe(&mut inventory, port, path, context)?;
        inventory.observations.insert(path.clone(), observation);
    }
    for include_pass in [false, true] {
        if include_pass && !port.has_includes() {
            break;
        }
        for entry in port.walk(include_pass) {
            context.check()?;
            if inventory.visited_entries >= MAX_VISITED {
                inventory.budget_exhausted = true;
                inventory.discovery_complete = false;
                break;
            }
            inventory.visited_entries += 1;
            if entry.scan_error {
                inventory.discovery_complete = false;
                inventory.skipped_entries += 1;
            }
            if !entry.is_file {
                continue;
            }
            let Some(path) = entry.path else {
                inventory.discovery_complete = false;
                inventory.skipped_entries += 1;
                continue;
            };
            // Only formats supported by native extraction, including Razor, Vue and HTML.
            // Never claim provider-wide inventory coverage.
            if !port.supported(&path)
                || port.restricted_reason(&path).is_some()
                || port.excluded(&path)
                || (include_pass && !port.included(&path))
                || inventory.observations.contains_key(&path)
            {
                continue;
            }
            if inventory.observations.len() >= MAX_FILES {
                inventory.budget_exhausted = true;
                inventory.discovery_complete = false;
                break;
            }
            let observation = observe(&mut inventory, port, &path, context)?;
            inventory.observations.insert(path, observation);
        }
        if inventory.budget_exhausted {
            break;
        }
    }
    context.check()?;
    port.recheck(context)?;
    context.check()?;
    context.report_progress("inventory", inventory.observations.len(), None, None);
    Ok(inventory)
}

fn unchecked(inventory: &mut Inventory, path: &str, code: &str, message: &str) -> Observation {
    inventory.skipped_entries += 1;
    inventory
        .file_diagnostics
        .push(serde_json::json!({"path":path,"stage":"inventory","code":code,"message":message}));
    Observation::Unchecked
}

pub fn observe<P: InventoryPort>(
    inventory: &mut Inventory,
    port: &P,
    relative: &str,
    context: &OperationContext,
) -> Result<Observation, Failure> {
    context.check()?;
    context.report_progress(
        "inventory",
        inventory.observations.len(),
        None,
        Some(relative),
    );
    let size = match port.metadata(relative) {
        Ok(SourceMetadata::Missing) => return Ok(Observation::Missing),
        Ok(SourceMetadata::Regular(size)) => size,
        Ok(SourceMetadata::Other) => u64::MAX,
        Err((code, message)) => return Ok(unchecked(inventory, relative, &code, &message)),
    };
    if size > MAX_SOURCE_BYTES as u64 {
        return Ok(unchecked(
            inventory,
            relative,
            "SOURCE_LIMIT",
            "Source is not a regular file or exceeds the extraction size limit",
        ));
    }
    if inventory.reserved_source_bytes + size > MAX_BYTES {
        inventory.budget_exhausted = true;
        inventory.discovery_complete = false;
        return Ok(unchecked(
            inventory,
            relative,
            "INVENTORY_LIMIT",
            "Aggregate source byte budget exhausted",
        ));
    }
    inventory.reserved_source_bytes += size;
    // Reserve the observed size and cap the actual read at that size. Growth
    // during reading cannot bypass the aggregate reservation.
    let bytes = match port.read(relative, size, context) {
        Ok(bytes) => bytes,
        Err(failure @ ("CANCELLED" | "TIMEOUT", _)) => return Err(failure),
        Err((code, message)) => {
            return Ok(unchecked(inventory, relative, code, message));
        }
    };
    if bytes.len() as u64 > size {
        return Ok(unchecked(
            inventory,
            relative,
            "SOURCE_LIMIT",
            "Source grew beyond the reserved read budget",
        ));
    }
    match source_hash(&bytes) {
        Ok(digest) => Ok(Observation::Content(digest)),
        Err((code, message)) => Ok(unchecked(inventory, relative, code, message)),
    }
}

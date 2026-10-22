//! Pure decisions used by the transactional publication coordinator.
use super::super::repository::{Incoming, framework};
use super::super::{opt_string, string};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(super) enum IncomingDisposition {
    Retain(Incoming),
    Recover {
        edge: Incoming,
        name: String,
        kind: String,
    },
}

/// Decide whether a clean indexed snapshot can be reused. The caller obtains
/// the snapshot inside its write transaction; this function has no I/O.
pub(super) fn is_unchanged(
    record: Option<&(String, Option<String>)>,
    bytes: Option<&[u8]>,
    config_changed: bool,
    force: bool,
    digest: impl FnOnce(&[u8]) -> String,
) -> bool {
    let clean = record.is_some_and(|(_, errors)| {
        errors.as_ref().is_none_or(|e| {
            serde_json::from_str::<Value>(e).is_ok_and(|v| v.as_array().is_some_and(Vec::is_empty))
        })
    });
    !config_changed
        && !force
        && ((bytes.is_none() && record.is_none())
            || (clean
                && bytes.is_some_and(|bytes| {
                    record.is_some_and(|(stored, _)| *stored == digest(bytes))
                })))
}

/// Reconcile one incoming edge against the new node identities without
/// touching SQLite. The coordinator checks cancellation between edges.
pub(super) fn incoming_disposition(
    mut edge: Incoming,
    provider_ids: &BTreeSet<String>,
    paired: &BTreeMap<String, String>,
) -> Result<IncomingDisposition, (&'static str, &'static str)> {
    let framework_template = edge
        .metadata
        .as_deref()
        .and_then(|metadata| serde_json::from_str::<Value>(metadata).ok())
        .and_then(|metadata| opt_string(&metadata, "refKind").map(str::to_owned))
        .is_some_and(|kind| framework::is_template_reference(&kind));
    if framework_template {
        let metadata: Value = edge
            .metadata
            .as_ref()
            .and_then(|s| serde_json::from_str(s).ok())
            .ok_or((
                "SYNC_LEGACY_REFERENCE",
                "Cannot recover framework template reference; index unchanged",
            ))?;
        let name = string(&metadata, "refName").map_err(|_| {
            (
                "SYNC_LEGACY_REFERENCE",
                "Framework template edge lacks original path; index unchanged",
            )
        })?;
        return Ok(IncomingDisposition::Recover {
            edge,
            name,
            kind: "framework_template".to_owned(),
        });
    }
    if provider_ids.contains(&edge.target) {
        return Ok(IncomingDisposition::Retain(edge));
    }
    if let Some(target) = paired.get(&edge.target) {
        edge.target = target.clone();
        return Ok(IncomingDisposition::Retain(edge));
    }
    let metadata: Value = edge
        .metadata
        .as_ref()
        .and_then(|s| serde_json::from_str(s).ok())
        .ok_or((
            "SYNC_LEGACY_REFERENCE",
            "Cannot recover original reference for an incoming edge; index unchanged",
        ))?;
    let name = string(&metadata, "refName").map_err(|_| {
        (
            "SYNC_LEGACY_REFERENCE",
            "Incoming edge lacks original reference metadata; index unchanged",
        )
    })?;
    let kind = opt_string(&metadata, "refKind")
        .unwrap_or(&edge.kind)
        .to_owned();
    Ok(IncomingDisposition::Recover { edge, name, kind })
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;

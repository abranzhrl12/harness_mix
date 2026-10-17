//! Compare contributing source files with hashes from the same graph read snapshot.
//! Matching files never certify index coverage, semantic correctness or freshness.
use super::project::Project;
use harness_domain::{Failure, execution::OperationContext, hash};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

pub(super) struct Expected {
    pub hash: Option<String>,
    pub reason: Option<&'static str>,
}

pub(super) fn indexed_files(
    db: &Connection,
    files: impl IntoIterator<Item = String>,
    context: &OperationContext,
) -> Result<BTreeMap<String, Expected>, Failure> {
    context.check()?;
    // Legacy/minimal indexes can still be queried, but cannot support a match.
    // Never execute a substituted view or publish the provider's error messages.
    let ordinary = db.query_row("SELECT count(*) FROM sqlite_schema WHERE name='files' AND type='table' AND upper(sql) LIKE 'CREATE TABLE%'", [], |r| r.get::<_, i64>(0)).unwrap_or(0) == 1;
    let mut statement = if ordinary {
        db.prepare("SELECT content_hash,errors FROM files WHERE path=?1 COLLATE BINARY")
            .ok()
    } else {
        None
    };
    let mut expected = BTreeMap::new();
    for path in files {
        context.check()?;
        let row = statement.as_mut().map(|query| {
            query
                .query_row([&path], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
                })
                .optional()
        });
        let item = match row {
            Some(Ok(Some((digest, errors)))) => {
                let valid_hash =
                    digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit());
                let clean = errors.as_ref().is_none_or(|e| {
                    serde_json::from_str::<Value>(e)
                        .ok()
                        .is_some_and(|v| v.as_array().is_some_and(Vec::is_empty))
                });
                Expected {
                    hash: valid_hash.then(|| digest.to_ascii_lowercase()),
                    reason: if !valid_hash {
                        Some("invalid_indexed_hash")
                    } else if !clean {
                        Some("index_extraction_errors")
                    } else {
                        None
                    },
                }
            }
            Some(Ok(None)) => Expected {
                hash: None,
                reason: Some("no_indexed_file_record"),
            },
            _ => Expected {
                hash: None,
                reason: Some("indexed_file_metadata_unavailable"),
            },
        };
        expected.insert(path, item);
    }
    // Configuration contributes to every native import edge; never certify
    // source hashes alone after its aliases changed or the config disappeared.
    let metadata_kind: Option<String> = db
        .query_row(
            "SELECT type FROM sqlite_schema WHERE name='project_metadata'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| ("INDEX", "Configuration provenance unavailable"))?;
    if let Some(kind) = metadata_kind {
        if kind != "table" {
            return Err((
                "INDEX",
                "Configuration provenance must be an ordinary table",
            ));
        }
        let stamp: Option<String> = db
            .query_row(
                "SELECT value FROM project_metadata WHERE key='native_tsconfig_v1'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| ("INDEX", "Configuration provenance unavailable"))?;
        if let Some(stamp) = stamp {
            if stamp.len() > 16 * 1024 {
                return Err(("INDEX", "Invalid configuration provenance"));
            }
            let stamp: Value = serde_json::from_str(&stamp)
                .map_err(|_| ("INDEX", "Invalid configuration provenance"))?;
            let mut contributors = vec![&stamp];
            if let Some(dependencies) = stamp.get("dependencies") {
                let dependencies = dependencies
                    .as_array()
                    .filter(|items| items.len() <= 7)
                    .ok_or(("INDEX", "Invalid configuration provenance"))?;
                contributors.extend(dependencies);
            }
            for stamp in contributors {
                let path = stamp["path"]
                    .as_str()
                    .ok_or(("INDEX", "Invalid configuration provenance"))?;
                let digest = stamp["hash"]
                    .as_str()
                    .ok_or(("INDEX", "Invalid configuration provenance"))?;
                if path.len() > 1000
                    || super::index_inventory::restricted(path)
                    || super::project::external_restricted(path)
                    || path.contains(['\\', ':'])
                    || path.split('/').any(|p| matches!(p, "" | "." | ".."))
                    || digest.len() != 64
                    || !digest.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err(("INDEX", "Invalid configuration provenance"));
                }
                expected.insert(
                    path.into(),
                    Expected {
                        hash: Some(digest.to_ascii_lowercase()),
                        reason: None,
                    },
                );
            }
        }
    }
    context.check()?;
    Ok(expected)
}

/// Decode only the selected edge's JSON provenance, never arbitrary paths from
/// another edge or a global index stamp. Legacy edges have no such field.
pub(super) fn export_contributors(
    raw: Option<&str>,
    observed: &mut BTreeMap<String, String>,
) -> Result<(), Failure> {
    let invalid = ("GRAPH_CONTRACT", "Invalid or conflicting export provenance");
    let Some(raw) = raw else { return Ok(()) };
    if raw.len() > 20 * 1024 {
        return Err(invalid);
    }
    let value: Value = serde_json::from_str(raw).map_err(|_| invalid)?;
    let items = value
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= 16)
        .ok_or(invalid)?;
    for item in items {
        let path = item["path"].as_str().ok_or(invalid)?;
        let digest = item["hash"].as_str().ok_or(invalid)?;
        if path.len() > 1000
            || path.chars().any(char::is_control)
            || path.contains(['\\', ':'])
            || path.split('/').any(|p| matches!(p, "" | "." | ".."))
            || super::index_inventory::restricted(path)
            || super::project::external_restricted(path)
            || digest.len() != 64
            || !digest.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(invalid);
        }
        let digest = digest.to_ascii_lowercase();
        if observed.get(path).is_some_and(|old| old != &digest) {
            return Err(invalid);
        }
        observed.insert(path.into(), digest);
        if observed.len() > 256 {
            return Err(("GRAPH_LIMIT", "Export provenance exceeds query budget"));
        }
    }
    Ok(())
}

#[derive(Clone)]
struct Observation {
    status: &'static str,
    reason: &'static str,
    hash: Option<String>,
}
fn unknown(reason: &'static str) -> Observation {
    Observation {
        status: "unknown",
        reason,
        hash: None,
    }
}

pub(super) struct Assessment {
    expected: BTreeMap<String, Expected>,
    before: BTreeMap<String, Observation>,
    started: Instant,
    reserved_bytes: usize,
}
impl Assessment {
    pub fn start(
        project: &Project,
        expected: BTreeMap<String, Expected>,
        context: &OperationContext,
    ) -> Result<Self, Failure> {
        let mut assessment = Self {
            expected,
            before: BTreeMap::new(),
            started: Instant::now(),
            reserved_bytes: 0,
        };
        for path in assessment.expected.keys().cloned().collect::<Vec<_>>() {
            let observed = assessment.observe(project, &path, context)?;
            assessment.before.insert(path, observed);
        }
        Ok(assessment)
    }

    fn observe(
        &mut self,
        project: &Project,
        path: &str,
        context: &OperationContext,
    ) -> Result<Observation, Failure> {
        context.check()?;
        if self.started.elapsed() >= Duration::from_secs(5) {
            return Ok(unknown("observation_time_budget"));
        }
        let absolute = match project.target_optional(path) {
            Ok(path) => path,
            Err(_) => return Ok(unknown("source_path_unavailable")),
        };
        let metadata = match std::fs::metadata(absolute) {
            Ok(metadata) if metadata.is_file() => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Observation {
                    status: "stale",
                    reason: "source_missing",
                    hash: None,
                });
            }
            _ => return Ok(unknown("source_unavailable")),
        };
        let size = metadata.len();
        // Newer CodeGraph versions hash a size-only stamp for oversized files.
        // Even if such a stamp matches, it cannot certify source content.
        if size > 1024 * 1024
            && self.expected[path].hash.as_deref()
                == Some(&hash(format!("codegraph:oversize:{size}").as_bytes()))
        {
            return Ok(unknown("oversize_index_marker"));
        }
        if size > 2 * 1024 * 1024 {
            return Ok(unknown("source_size_budget"));
        }
        if self.reserved_bytes.saturating_add(size as usize) > 64 * 1024 * 1024 {
            return Ok(unknown("observation_byte_budget"));
        }
        // Reserve even if the read fails. Both observations share this budget.
        self.reserved_bytes += size as usize;
        let bytes = match project.bytes_with_limit(path, context, size) {
            Ok(bytes) => bytes,
            Err(error @ ("CANCELLED" | "TIMEOUT", _)) => return Err(error),
            Err(("STALE", _)) => {
                return Ok(Observation {
                    status: "stale",
                    reason: "source_changed_during_read",
                    hash: None,
                });
            }
            Err(_) => return Ok(unknown("source_read_unavailable")),
        };
        if self.started.elapsed() >= Duration::from_secs(5) {
            return Ok(unknown("observation_time_budget"));
        }
        if std::str::from_utf8(&bytes).is_err() {
            return Ok(unknown("non_utf8_source"));
        }
        let digest = hash(&bytes);
        let expected = &self.expected[path];
        if let Some(reason) = expected.reason {
            return Ok(unknown(reason));
        }
        Ok(Observation {
            status: if expected.hash.as_ref() == Some(&digest) {
                "matches"
            } else {
                "stale"
            },
            reason: if expected.hash.as_ref() == Some(&digest) {
                "indexed_hash_matches"
            } else {
                "source_changed_since_index"
            },
            hash: Some(digest),
        })
    }

    pub fn finish(
        mut self,
        project: &Project,
        definition_hashes: &BTreeMap<String, String>,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        let mut records = Vec::new();
        let (mut matching, mut stale, mut unchecked) = (0, 0, 0);
        for path in self.expected.keys().cloned().collect::<Vec<_>>() {
            let after = self.observe(project, &path, context)?;
            let before = &self.before[&path];
            let local_changed = definition_hashes.get(&path).is_some_and(|hash| {
                self.expected[&path]
                    .hash
                    .as_ref()
                    .is_some_and(|expected| hash != expected)
            });
            let (status, reason) = if before.status == "stale" {
                ("stale", before.reason)
            } else if after.status == "stale" {
                ("stale", after.reason)
            } else if local_changed {
                ("stale", "source_changed_during_local_definition_read")
            } else if before.status == "matches"
                && after.status == "matches"
                && before.hash == after.hash
            {
                ("matches", "indexed_hash_matches_both_observations")
            } else {
                (
                    "unknown",
                    if before.status == "unknown" {
                        before.reason
                    } else {
                        after.reason
                    },
                )
            };
            match status {
                "matches" => matching += 1,
                "stale" => stale += 1,
                _ => unchecked += 1,
            }
            records.push(json!({"path":path,"status":status,"reason":reason}));
        }
        context.check()?;
        let status = if stale > 0 {
            "stale"
        } else if unchecked > 0 || records.is_empty() {
            "unknown"
        } else {
            "matches_observed_files"
        };
        Ok(
            json!({"status":status,"files":records,"matchingFiles":matching,"staleFiles":stale,"uncheckedFiles":unchecked,
            "scope":"files contributing nodes to the returned SQLite snapshot; observed before and after local definition reads",
            "checkedAt":chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "certifiesIndexFreshness":false,"unindexedFilesChecked":false,"wholeWorkspaceChecked":false,"relationsRevalidated":false,
            "derivedForThisResponse":true,"budgets":{"maxBytesBothObservations":67108864,"maxFileBytes":2097152,"maxMilliseconds":5000},
            "notice":"Matching content does not prove index completeness, relation validity, unindexed changes or a coherent whole-workspace snapshot; intermediate reverted changes can be missed."}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn expected() -> BTreeMap<String, Expected> {
        BTreeMap::from([(
            "source.rs".into(),
            Expected {
                hash: Some(hash(b"original")),
                reason: None,
            },
        )])
    }
    #[test]
    fn changes_reversions_and_intermediate_definition_reads_cannot_restore_a_match() {
        let root = tempfile::tempdir().unwrap();
        let project = Project::new(root.path()).unwrap();
        let context = OperationContext::default();
        std::fs::write(root.path().join("source.rs"), "original").unwrap();
        let check = Assessment::start(&project, expected(), &context).unwrap();
        std::fs::write(root.path().join("source.rs"), "changed").unwrap();
        assert_eq!(
            check.finish(&project, &BTreeMap::new(), &context).unwrap()["status"],
            "stale"
        );
        let check = Assessment::start(&project, expected(), &context).unwrap();
        std::fs::write(root.path().join("source.rs"), "original").unwrap();
        assert_eq!(
            check.finish(&project, &BTreeMap::new(), &context).unwrap()["status"],
            "stale"
        );
        let check = Assessment::start(&project, expected(), &context).unwrap();
        let intermediate = BTreeMap::from([("source.rs".into(), hash(b"changed"))]);
        let result = check.finish(&project, &intermediate, &context).unwrap();
        assert_eq!(result["status"], "stale");
        assert_eq!(
            result["files"][0]["reason"],
            "source_changed_during_local_definition_read"
        );
    }
    #[test]
    fn exhausted_budget_invalid_utf8_and_empty_coverage_remain_unknown() {
        let root = tempfile::tempdir().unwrap();
        let project = Project::new(root.path()).unwrap();
        let context = OperationContext::default();
        std::fs::write(root.path().join("source.rs"), "original").unwrap();
        let mut check = Assessment::start(&project, expected(), &context).unwrap();
        check.reserved_bytes = 64 * 1024 * 1024;
        let result = check.finish(&project, &BTreeMap::new(), &context).unwrap();
        assert_eq!(result["status"], "unknown");
        assert_eq!(result["files"][0]["reason"], "observation_byte_budget");
        let check = Assessment::start(&project, expected(), &context).unwrap();
        std::fs::write(root.path().join("source.rs"), [0xff]).unwrap();
        let result = check.finish(&project, &BTreeMap::new(), &context).unwrap();
        assert_eq!(result["status"], "unknown");
        assert_eq!(result["files"][0]["reason"], "non_utf8_source");
        let empty = Assessment::start(&project, BTreeMap::new(), &context)
            .unwrap()
            .finish(&project, &BTreeMap::new(), &context)
            .unwrap();
        assert_eq!(empty["status"], "unknown");
        assert_eq!(empty["certifiesIndexFreshness"], false);
    }
}

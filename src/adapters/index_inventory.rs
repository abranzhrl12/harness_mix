//! Conservative inventory for the first native CodeGraph synchronization stage.
use super::{
    index_sync::{self, Observation},
    project::{self, Project},
};
use harness_domain::{Failure, execution::OperationContext};
use serde::Serialize;
use std::{collections::BTreeMap, path::Path};

const MAX_VISITED: usize = 40_000;
const MAX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Inventory {
    #[serde(skip)]
    pub observations: BTreeMap<String, Observation>,
    pub discovery_complete: bool,
    pub visited_entries: usize,
    pub reserved_source_bytes: u64,
    pub skipped_entries: usize,
    pub restricted_indexed_files: usize,
    // One policy category per restricted entry; never expose its path or hash.
    pub restricted_indexed_reasons: BTreeMap<&'static str, usize>,
    pub budget_exhausted: bool,
    pub scope_excluded: Vec<String>,
    pub scope: serde_json::Value,
    pub file_diagnostics: Vec<serde_json::Value>,
}

pub(super) fn excluded(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        ".git"
            | ".hg"
            | ".svn"
            | ".codegraph"
            | "node_modules"
            | "target"
            | "vendor"
            | "dist"
            | "build"
            | "coverage"
            | ".venv"
            | "venv"
            | "obj"
    )
}

pub(super) fn restricted(path: &str) -> bool {
    restricted_reason(path).is_some()
}

fn restricted_reason(path: &str) -> Option<&'static str> {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    if normalized == ".agents/harness/proposals"
        || normalized.starts_with(".agents/harness/proposals/")
    {
        return Some("proposal_storage");
    }
    project::external_restricted_reason(path).or_else(|| project::external_sensitive_reason(path))
}

pub(super) fn scan(
    project: &Project,
    tracked: &[String],
    context: &OperationContext,
) -> Result<Inventory, Failure> {
    context.report_progress("inventory", 0, None, None);
    let scope = super::index_store::scope::Snapshot::load(project, context)?;
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
        scope: scope.stamp(),
        file_diagnostics: Vec::new(),
    };
    // Check each tracked path directly. Discovery exclusions must never turn an
    // old provider record into an inferred deletion. This is NOT full ScopeIgnore.
    for path in tracked {
        context.check()?;
        if let Some(reason) = restricted_reason(path) {
            inventory.restricted_indexed_files += 1;
            *inventory
                .restricted_indexed_reasons
                .entry(reason)
                .or_default() += 1;
            continue;
        }
        if scope.excluded(path) {
            inventory.scope_excluded.push(path.clone());
            continue;
        }
        let observation = inventory.observe(project, path, context)?;
        inventory.observations.insert(path.clone(), observation);
    }
    for include_pass in [false, true] {
        if include_pass && !scope.has_includes {
            break;
        }
        let mut builder = ignore::WalkBuilder::new(&project.root);
        builder
            .follow_links(false)
            .hidden(false)
            .parents(false)
            .git_ignore(!include_pass)
            .git_exclude(!include_pass)
            .git_global(false)
            .require_git(false)
            .ignore(false)
            .sort_by_file_path(|a, b| a.cmp(b))
            .filter_entry(|entry| {
                entry.depth() == 0 || !excluded(&entry.file_name().to_string_lossy())
            });
        for entry in builder.build() {
            context.check()?;
            if inventory.visited_entries >= MAX_VISITED {
                inventory.budget_exhausted = true;
                inventory.discovery_complete = false;
                break;
            }
            inventory.visited_entries += 1;
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    inventory.discovery_complete = false;
                    inventory.skipped_entries += 1;
                    continue;
                }
            };
            if entry.error().is_some() {
                inventory.discovery_complete = false;
                inventory.skipped_entries += 1;
            }
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let Some(path) = entry
                .path()
                .strip_prefix(&project.root)
                .ok()
                .and_then(Path::to_str)
            else {
                inventory.discovery_complete = false;
                inventory.skipped_entries += 1;
                continue;
            };
            let path = path.replace('\\', "/");
            // Only formats supported by native extraction, including Razor, Vue and HTML.
            // Never claim provider-wide inventory coverage.
            if super::index_extract::language(&path).is_none()
                || restricted(&path)
                || scope.excluded(&path)
                || (include_pass && !scope.included(&path))
                || inventory.observations.contains_key(&path)
            {
                continue;
            }
            if inventory.observations.len() >= index_sync::MAX_FILES {
                inventory.budget_exhausted = true;
                inventory.discovery_complete = false;
                break;
            }
            let observation = inventory.observe(project, &path, context)?;
            inventory.observations.insert(path, observation);
        }
        if inventory.budget_exhausted {
            break;
        }
    }
    scope.recheck(project, context)?;
    context.report_progress("inventory", inventory.observations.len(), None, None);
    Ok(inventory)
}

impl Inventory {
    fn unchecked(&mut self, path: &str, code: &str, message: &str) -> Observation {
        self.skipped_entries += 1;
        self.file_diagnostics.push(
            serde_json::json!({"path":path,"stage":"inventory","code":code,"message":message}),
        );
        Observation::Unchecked
    }

    fn observe(
        &mut self,
        project: &Project,
        relative: &str,
        context: &OperationContext,
    ) -> Result<Observation, Failure> {
        context.check()?;
        context.report_progress("inventory", self.observations.len(), None, Some(relative));
        let path = match project.target_optional(relative) {
            Ok(path) => path,
            Err((code, message)) => {
                return Ok(self.unchecked(relative, code, message));
            }
        };
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Observation::Missing);
            }
            Err(error) => {
                return Ok(self.unchecked(relative, "IO_METADATA", &error.to_string()));
            }
        };
        if !metadata.is_file() || metadata.len() > index_sync::MAX_SOURCE_BYTES as u64 {
            return Ok(self.unchecked(
                relative,
                "SOURCE_LIMIT",
                "Source is not a regular file or exceeds the extraction size limit",
            ));
        }
        if self.reserved_source_bytes + metadata.len() > MAX_BYTES {
            self.budget_exhausted = true;
            self.discovery_complete = false;
            return Ok(self.unchecked(
                relative,
                "INVENTORY_LIMIT",
                "Aggregate source byte budget exhausted",
            ));
        }
        self.reserved_source_bytes += metadata.len();
        #[cfg(windows)]
        let _pins = match super::edit_commit::pin_directory(
            project,
            path.parent().ok_or(("PATH", "Missing source parent"))?,
        ) {
            Ok(pins) => pins,
            Err((code, message)) => {
                return Ok(self.unchecked(relative, code, message));
            }
        };
        // Reserve the observed size and cap the actual read at that size. Growth
        // during reading cannot bypass the aggregate reservation.
        let bytes = match project.bytes_with_limit(relative, context, metadata.len()) {
            Ok(bytes) => bytes,
            Err(failure @ ("CANCELLED" | "TIMEOUT", _)) => return Err(failure),
            Err((code, message)) => {
                return Ok(self.unchecked(relative, code, message));
            }
        };
        match index_sync::source_hash(&bytes) {
            Ok(digest) => Ok(Observation::Content(digest)),
            Err((code, message)) => Ok(self.unchecked(relative, code, message)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unreadable_encoding_has_a_file_diagnostic_and_is_not_missing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("invalid.rs"), [0xff, 0xfe]).unwrap();
        let project = Project::new(dir.path()).unwrap();
        let inventory = scan(&project, &[], &OperationContext::default()).unwrap();
        assert!(matches!(
            inventory.observations["invalid.rs"],
            Observation::Unchecked
        ));
        assert_eq!(inventory.file_diagnostics[0]["path"], "invalid.rs");
        assert_eq!(inventory.file_diagnostics[0]["code"], "SYNC_UNCHECKED");
    }
    #[test]
    fn restricted_policy_counts_do_not_read_or_disclose_paths() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project::new(dir.path()).unwrap();
        let tracked: Vec<String> = [
            "NUXT.CONFIG.ts",
            "vite.config.ts",
            ".ENV/private.rs",
            ".git/internal.rs",
            "key.pem",
            "appsettings.production.json",
            ".npmrc",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let inventory = scan(&project, &tracked, &OperationContext::default()).unwrap();
        assert_eq!(inventory.restricted_indexed_files, 7);
        assert!(inventory.observations.is_empty());
        let value = serde_json::to_value(inventory).unwrap();
        assert_eq!(value["restrictedIndexedReasons"]["config_filename"], 2);
        for reason in [
            "sensitive_path_prefix",
            "version_control_directory",
            "private_key_extension",
            "appsettings_filename",
            "npmrc_filename",
        ] {
            assert_eq!(value["restrictedIndexedReasons"][reason], 1);
        }
        for path in tracked {
            assert!(!value.to_string().contains(&path));
        }
        assert!(!restricted("src/profile.ts"));
        assert!(!restricted("src/config.ts"));
        assert!(restricted("SRC\\NUXT.CONFIG.TS"));
    }
    #[test]
    fn aggregate_budget_never_turns_an_existing_source_into_a_deletion() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("source.rs"), "fn source() {}").unwrap();
        let project = Project::new(dir.path()).unwrap();
        let mut inventory = Inventory {
            observations: BTreeMap::new(),
            discovery_complete: true,
            visited_entries: 0,
            reserved_source_bytes: MAX_BYTES,
            skipped_entries: 0,
            restricted_indexed_files: 0,
            restricted_indexed_reasons: BTreeMap::new(),
            budget_exhausted: false,
            scope_excluded: Vec::new(),
            scope: serde_json::Value::Null,
            file_diagnostics: Vec::new(),
        };
        let observed = inventory
            .observe(&project, "source.rs", &OperationContext::default())
            .unwrap();
        assert!(matches!(observed, Observation::Unchecked));
        assert!(inventory.budget_exhausted);
        assert!(!inventory.discovery_complete);
        assert_eq!(inventory.reserved_source_bytes, MAX_BYTES);
    }
}

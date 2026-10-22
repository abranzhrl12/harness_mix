//! Filesystem inventory adapter. Bounded selection and budget policy live in application.
use super::project::{self, Project};
pub(super) use harness_application::indexing::inventory::Inventory;
use harness_application::ports::indexing::{DiscoveredEntry, InventoryPort, SourceMetadata};
use harness_domain::{Failure, execution::OperationContext};
use std::path::Path;

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
    context.check()?;
    let scope = super::index_store::scope::Snapshot::load(project, context)?;
    harness_application::indexing::inventory::scan(
        &NativeInventory { project, scope },
        tracked,
        context,
    )
}

struct NativeInventory<'a> {
    project: &'a Project,
    scope: super::index_store::scope::Snapshot,
}
impl InventoryPort for NativeInventory<'_> {
    fn scope(&self) -> serde_json::Value {
        self.scope.stamp()
    }
    fn has_includes(&self) -> bool {
        self.scope.has_includes
    }
    fn excluded(&self, path: &str) -> bool {
        self.scope.excluded(path)
    }
    fn included(&self, path: &str) -> bool {
        self.scope.included(path)
    }
    fn restricted_reason(&self, path: &str) -> Option<&'static str> {
        restricted_reason(path)
    }
    fn supported(&self, path: &str) -> bool {
        super::index_extract::language(path).is_some()
    }
    fn walk(&self, include_pass: bool) -> Box<dyn Iterator<Item = DiscoveredEntry>> {
        let root = self.project.root.clone();
        let mut builder = ignore::WalkBuilder::new(&root);
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
        Box::new(builder.build().map(move |entry| {
            match entry {
                Err(_) => DiscoveredEntry {
                    path: None,
                    is_file: false,
                    scan_error: true,
                },
                Ok(entry) => DiscoveredEntry {
                    path: entry
                        .path()
                        .strip_prefix(&root)
                        .ok()
                        .and_then(Path::to_str)
                        .map(|p| p.replace('\\', "/")),
                    is_file: entry.file_type().is_some_and(|kind| kind.is_file()),
                    scan_error: entry.error().is_some(),
                },
            }
        }))
    }
    fn metadata(&self, relative: &str) -> Result<SourceMetadata, (String, String)> {
        let path = self
            .project
            .target_optional(relative)
            .map_err(|(c, m)| (c.to_owned(), m.to_owned()))?;
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_file() => Ok(SourceMetadata::Regular(m.len())),
            Ok(_) => Ok(SourceMetadata::Other),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SourceMetadata::Missing),
            Err(e) => Err(("IO_METADATA".into(), e.to_string())),
        }
    }
    fn read(
        &self,
        relative: &str,
        limit: u64,
        context: &OperationContext,
    ) -> Result<Vec<u8>, Failure> {
        #[cfg(windows)]
        let _pins = {
            let path = self.project.target_optional(relative)?;
            super::edit_commit::pin_directory(
                self.project,
                path.parent().ok_or(("PATH", "Missing source parent"))?,
            )?
        };
        self.project.bytes_with_limit(relative, context, limit)
    }
    fn recheck(&self, context: &OperationContext) -> Result<(), Failure> {
        self.scope.recheck(self.project, context)
    }
}

#[cfg(test)]
#[path = "index_inventory_tests.rs"]
mod tests;

//! Narrow support for the CLI's project initialization transaction.

use std::path::{Path, PathBuf};

use crate::{lock::SyncLock, sync};

/// A checked project root held under the skills synchronization lock.
pub struct ProjectSupport {
    root: PathBuf,
    _lock: SyncLock,
}

impl ProjectSupport {
    /// Resolve a project root without acquiring the synchronization lock.
    pub fn checked_root(root: &Path) -> Result<PathBuf, String> {
        crate::storage::checked_root(root)
    }

    /// Check the root and acquire the lock for an initialization workflow.
    pub fn lock(root: &Path) -> Result<Self, String> {
        let root = crate::storage::checked_root(root)?;
        let lock = SyncLock::acquire(&root)?;
        Ok(Self { root, _lock: lock })
    }

    /// Produce a read-only synchronization plan for initialization checks.
    pub fn preview(root: &Path) -> Result<crate::Report, String> {
        sync::sync(root, true, false)
    }

    /// Read the current auto-sync preference for initialization checks.
    pub fn preferences(root: &Path) -> Result<Option<crate::Preferences>, String> {
        crate::preferences::read_preferences(root)
    }

    /// Read a regular project file without following symlinks.
    pub fn read_file(root: &Path, relative: &Path) -> Result<Option<Vec<u8>>, String> {
        crate::storage::read_safe_file(root, relative)
    }

    /// Synchronize bundled skill files while this value holds the lock.
    pub fn sync(&self) -> Result<crate::Report, String> {
        sync::sync(&self.root, false, false)
    }

    /// Create a project file without replacing an existing file.
    pub fn write_new_json(&self, relative: &str, value: &serde_json::Value) -> Result<(), String> {
        crate::storage::write_new_json(&self.root, relative, value)
    }

    /// Set the opt-in startup synchronization preference.
    pub fn set_auto_sync(&self, enabled: bool) -> Result<bool, String> {
        crate::preferences::set_auto_sync(&self.root, enabled)
    }
}

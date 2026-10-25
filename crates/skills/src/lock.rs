use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
};

use crate::storage::{checked_root, create_safe_parents, safe_metadata};

const LOCK_PATH: &str = ".agents/.harness-skills-sync.lock";

pub(crate) struct SyncLock {
    path: PathBuf,
}

impl SyncLock {
    pub(crate) fn acquire(root: &Path) -> Result<Self, String> {
        let root = checked_root(root)?;
        let relative = Path::new(LOCK_PATH);
        let parent = create_safe_parents(&root, relative)?;
        if safe_metadata(&root, relative)?.is_some() {
            return Err(format!(
                "another skill sync owns {}; if no sync process is active, remove the stale lock after checking",
                relative.display()
            ));
        }
        let path = parent.join(".harness-skills-sync.lock");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| format!("cannot acquire skill sync lock: {error}"))?;
        Ok(Self { path })
    }
}

impl Drop for SyncLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

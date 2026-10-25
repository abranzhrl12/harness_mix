use harness_domain::Failure;
use std::{fs::File, path::Path};

/// Platform boundary for pinning a project directory and its ancestors.
/// Windows implementations must hold every ancestor against rename/deletion.
pub trait DirectoryPinner: Send + Sync {
    fn pin_directory(&self, root: &Path, path: &Path) -> Result<Vec<File>, Failure>;
}

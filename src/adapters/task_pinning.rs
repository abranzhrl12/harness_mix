//! Preserve the native directory-pinning policy for durable jobs.
use harness_domain::Failure;
use harness_jobs::DirectoryPinner;
use std::{fs::File, path::Path};

pub(super) struct NativeDirectoryPinner;

impl DirectoryPinner for NativeDirectoryPinner {
    fn pin_directory(&self, root: &Path, path: &Path) -> Result<Vec<File>, Failure> {
        #[cfg(windows)]
        {
            let project = super::project::Project::new(root)
                .map_err(|_| ("TASK_STORE_PATH", "Unsafe background job storage path"))?;
            super::edit_commit::pin_directory(&project, path)
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let _ = root;
            File::options()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path)
                .map(|file| vec![file])
                .map_err(|_| ("TASK_STORE_PATH", "Unsafe background job storage path"))
        }
        #[cfg(not(any(windows, unix)))]
        {
            let _ = root;
            File::open(path)
                .map(|file| vec![file])
                .map_err(|_| ("TASK_STORE_PATH", "Unsafe background job storage path"))
        }
    }
}

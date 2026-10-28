//! Cross-process serialization of one logical index publication.
//! Separate from autosync.lock (watcher lifetime) and SQLite's own writer lock.
use super::{INDEX, INDEX_BUSY, Project, sync_diagnostic};
use harness_domain::{Failure, execution::OperationContext};
use std::{
    fs::{File, TryLockError},
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::HANDLE,
    Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle},
};

const MAX_WAIT: Duration = Duration::from_secs(30);

/// The OS releases the lock when this handle closes, including process termination.
/// Keep this guard outside writer connections so SQLite closes before it is released.
pub(super) struct WriteGuard {
    _file: File,
    _directories: Vec<File>,
}

impl WriteGuard {
    pub(super) fn acquire(
        root: &Path,
        context: &OperationContext,
    ) -> Result<Self, Failure> {
        let project = Project::new(root).map_err(|_| ("PATH", "Workspace unavailable"))?;
        sync_diagnostic::begin("<writer-coordinator>", &project.root.join(".codegraph/codegraph.db"));
        sync_diagnostic::stage("wait_writer_coordinator");
        context.check()?;
        // Existing indexes only. Pin ancestors and refuse substitutions exactly as
        // the SQLite writer does; all aliases of a directory open the same lock.
        project.target(".codegraph/codegraph.db")?;
        let directory = project.target(".codegraph")?;
        let directories = super::super::edit_commit::pin_directory(&project, &directory)?;
        let path = project.target_optional(".codegraph/index-write.lock")?;
        let file = File::options()
            .read(true).write(true).create(true).truncate(false)
            .share_mode(3).custom_flags(0x0020_0000)
            .open(path)
            .map_err(|_| INDEX)?;
        let mut identity = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut identity) }
            .map_err(|_| INDEX)?;
        if identity.nNumberOfLinks != 1 || identity.dwFileAttributes & 0x400 != 0
            || !file.metadata().map_err(|_| INDEX)?.is_file()
            || file.metadata().map_err(|_| INDEX)?.len() != 0
        {
            return Err(("PATH", "Unsafe index writer coordination lock"));
        }
        let started = Instant::now();
        let mut delay = Duration::from_millis(10);
        let mut notified = false;
        loop {
            context.check()?;
            match file.try_lock() {
                Ok(()) => return Ok(Self { _file: file, _directories: directories }),
                Err(TryLockError::WouldBlock) => {
                    if !notified {
                        context.report_progress("waiting_index_writer", 0, None, None);
                        notified = true;
                    }
                    if started.elapsed() >= MAX_WAIT {
                        return Err(INDEX_BUSY);
                    }
                    std::thread::sleep(delay.min(MAX_WAIT.saturating_sub(started.elapsed())));
                    delay = (delay * 2).min(Duration::from_millis(50));
                }
                Err(TryLockError::Error(_)) => return Err(INDEX),
            }
        }
    }
}

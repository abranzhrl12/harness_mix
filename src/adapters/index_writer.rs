//! A pinned writer connection reused within one explicit sync operation.
use super::{Failure, INDEX, OperationContext, Project, sql};
use rusqlite::{Connection, OpenFlags};
use std::{
    fs::File,
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    time::Duration,
};
use windows::Win32::{
    Foundation::HANDLE,
    Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle},
};

pub(super) struct Writer {
    // On uncertain rollback, close SQLite before releasing protective handles.
    pub(super) db: Connection,
    sidecars: Vec<File>,
    pin: File,
    _directories: Vec<File>,
}

impl Drop for Writer {
    fn drop(&mut self) {
        // Fold the write-ahead log back into the database before releasing the
        // sidecars. A passive checkpoint never blocks and keeps the next
        // publication from paying for a large log.
        if self.db.is_autocommit() {
            let _ = self.db.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
            self.sidecars.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_close_can_remove_wal_and_shm_without_delete_retries() {
        let root = tempfile::tempdir().unwrap();
        let context = OperationContext::default();
        super::super::initialize(root.path(), &context).unwrap();
        let project = Project::new(root.path()).unwrap();
        let writer = Writer::open(&project, &context).unwrap();
        writer
            .db
            .execute(
                "INSERT INTO project_metadata(key,value,updated_at) VALUES('close-test','ok',0)",
                [],
            )
            .unwrap();
        assert!(root.path().join(".codegraph/codegraph.db-wal").exists());
        drop(writer);
        assert!(!root.path().join(".codegraph/codegraph.db-wal").exists());
        assert!(!root.path().join(".codegraph/codegraph.db-shm").exists());
    }
}

impl Writer {
    pub(super) fn open(project: &Project, context: &OperationContext) -> Result<Self, Failure> {
        context.check()?;
        let path = project
            .target(".codegraph/codegraph.db")
            .map_err(|_| INDEX)?;
        for suffix in ["-wal", "-shm", "-journal"] {
            project
                .target_optional(&format!(".codegraph/codegraph.db{suffix}"))
                .map_err(|_| INDEX)?;
        }
        let _pins = super::super::edit_commit::pin_directory(project, path.parent().ok_or(INDEX)?)?;
        // Deny DELETE sharing so SQLite's path cannot be replaced after validation.
        let pin = File::options()
            .read(true)
            .share_mode(3)
            .custom_flags(0x0020_0000)
            .open(&path)
            .map_err(|_| INDEX)?;
        let metadata = pin.metadata().map_err(|_| INDEX)?;
        if !metadata.is_file()
            || metadata.len() > 512 * 1024 * 1024
            || metadata.file_attributes() & 0x400 != 0
        {
            return Err(INDEX);
        }
        let mut identity = BY_HANDLE_FILE_INFORMATION::default();
        unsafe {
            GetFileInformationByHandle(HANDLE(pin.as_raw_handle()), &mut identity)
                .map_err(|_| INDEX)?;
        }
        if identity.nNumberOfLinks != 1 {
            return Err(("PATH", "Hard-linked indexes cannot be updated"));
        }
        // Pin deterministic sidecar names too: an attacker must not substitute a
        // junction/symlink between validation and SQLite opening a writable WAL.
        // WAL mode doesn't need to unlink these files at commit. They may remain
        // after closing; this is a valid SQLite WAL layout, not source modification.
        let mut sidecar_pins = Vec::new();
        for suffix in ["-wal", "-shm", "-journal"] {
            let sidecar = project.target_optional(&format!(".codegraph/codegraph.db{suffix}"))?;
            let options = || {
                let mut options = File::options();
                options
                    .read(true)
                    .write(true)
                    .share_mode(3)
                    .custom_flags(0x0020_0000);
                options
            };
            let guard = match options().create_new(true).open(&sidecar) {
                Ok(guard) => guard,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    options().open(&sidecar).map_err(|_| INDEX)?
                }
                Err(_) => return Err(INDEX),
            };
            let meta = guard.metadata().map_err(|_| INDEX)?;
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            unsafe {
                GetFileInformationByHandle(HANDLE(guard.as_raw_handle()), &mut info)
                    .map_err(|_| INDEX)?;
            }
            if !meta.is_file() || meta.file_attributes() & 0x400 != 0 || info.nNumberOfLinks != 1 {
                return Err(("PATH", "Linked SQLite sidecars cannot be updated"));
            }
            sidecar_pins.push(guard);
        }
        let db = sql(Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        ))?;
        sql(db.busy_timeout(Duration::from_millis(100)))?;
        let journal: String = sql(db.query_row("PRAGMA journal_mode", [], |r| r.get(0)))?;
        if journal != "wal" {
            return Err((
                "SYNC_JOURNAL",
                "Native publication requires an existing WAL-mode CodeGraph index",
            ));
        }
        sql(db.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH, 1024 * 1024))?;
        sql(db.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_SQL_LENGTH, 16384))?;
        sql(db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF; PRAGMA synchronous=FULL; PRAGMA temp_store=MEMORY; PRAGMA cache_size=-4096; PRAGMA wal_autocheckpoint=8000;"))?;
        Ok(Self {
            db,
            sidecars: sidecar_pins,
            pin,
            _directories: _pins,
        })
    }
    pub(super) fn recheck(&self, context: &OperationContext) -> Result<(), Failure> {
        context.check()?;
        for (file, index) in
            std::iter::once((&self.pin, true)).chain(self.sidecars.iter().map(|f| (f, false)))
        {
            let metadata = file.metadata().map_err(|_| INDEX)?;
            let mut identity = BY_HANDLE_FILE_INFORMATION::default();
            unsafe {
                GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut identity)
                    .map_err(|_| INDEX)?;
            }
            if !metadata.is_file()
                || metadata.file_attributes() & 0x400 != 0
                || identity.nNumberOfLinks != 1
            {
                return Err(("PATH", "Linked SQLite files cannot be updated"));
            }
            if index && metadata.len() > 512 * 1024 * 1024 {
                return Err(INDEX);
            }
        }
        Ok(())
    }
}

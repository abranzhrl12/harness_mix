//! Fixed-path, single-writer SQLite journal for managed verification jobs.
use harness_domain::Failure;
use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const IO: Failure = (
    "TASK_STORE",
    "Cannot persist background jobs; no new work admitted",
);
const PATH: Failure = ("TASK_STORE_PATH", "Unsafe background job storage path");
const FORMAT: Failure = (
    "TASK_STORE_FORMAT",
    "Invalid background job database; existing data was not replaced",
);
const BUSY: Failure = (
    "TASK_STORE_BUSY",
    "Another background job writer owns this project",
);
const MAX_SNAPSHOT: usize = 16 * 1024 * 1024;
const MAX_FILE: u64 = 64 * 1024 * 1024;

pub(super) struct Store {
    // SQLite closes while the database and directory pins still exist.
    db: Connection,
    _files: Vec<File>,
    _writer: File,
    _directories: Vec<File>,
    directory: PathBuf,
}

fn sql<T>(result: rusqlite::Result<T>) -> Result<T, Failure> {
    result.map_err(|_| IO)
}

fn linked(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

fn inspect(file: &File) -> Result<(), Failure> {
    let meta = file.metadata().map_err(|_| PATH)?;
    if !meta.is_file() || linked(&meta) || meta.len() > MAX_FILE {
        return Err(PATH);
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle},
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: the borrowed file handle is live and the output structure is initialized.
        unsafe {
            GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)
                .map_err(|_| PATH)?;
        }
        if info.nNumberOfLinks != 1 {
            return Err(PATH);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            return Err(PATH);
        }
    }
    Ok(())
}

fn file_options(exclusive: bool) -> OpenOptions {
    let mut options = File::options();
    options.read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .custom_flags(0x0020_0000)
            .share_mode(if exclusive { 0 } else { 3 });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let _ = exclusive;
        options
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .mode(0o600);
    }
    options
}

fn checked_file(path: &Path, create: bool) -> Result<File, Failure> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if linked(&meta) || !meta.is_file() {
            return Err(PATH);
        }
    }
    let file = if create {
        match file_options(false).create_new(true).open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                file_options(false).open(path).map_err(|_| PATH)?
            }
            Err(_) => return Err(IO),
        }
    } else {
        file_options(false).open(path).map_err(|_| PATH)?
    };
    inspect(&file)?;
    Ok(file)
}

fn sidecar_file(path: &Path) -> Result<File, Failure> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if linked(&meta) || !meta.is_file() {
            return Err(PATH);
        }
    }
    let options = || {
        let mut options = file_options(false);
        // SQLite must be able to delete a hot rollback journal during recovery.
        // The DB and ancestor pins retain deny-rename protection separately.
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(7);
        }
        options
    };
    let file = match options().create_new(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            options().open(path).map_err(|_| PATH)?
        }
        Err(_) => return Err(IO),
    };
    inspect(&file)?;
    Ok(file)
}

/// None means at least one managed directory does not exist; no directories are created.
fn directory(root: &Path, create: bool) -> Result<Option<(PathBuf, Vec<File>)>, Failure> {
    let root_meta = fs::symlink_metadata(root).map_err(|_| PATH)?;
    if !root_meta.is_dir() || linked(&root_meta) {
        return Err(PATH);
    }
    let canonical = root.canonicalize().map_err(|_| PATH)?;
    let mut current = canonical.clone();
    #[cfg(windows)]
    let project = super::project::Project::new(&canonical).map_err(|_| PATH)?;
    #[cfg(windows)]
    let mut pins = super::edit_commit::pin_directory(&project, &current).map_err(|_| PATH)?;
    #[cfg(not(windows))]
    let mut pins = vec![File::open(&current).map_err(|_| PATH)?];
    for component in [".agents", "harness", "jobs"] {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if !create {
                    return Ok(None);
                }
                match fs::create_dir(&current) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(_) => return Err(IO),
                }
            }
            Err(_) => return Err(PATH),
            Ok(_) => {}
        }
        let meta = fs::symlink_metadata(&current).map_err(|_| PATH)?;
        if !meta.is_dir() || linked(&meta) {
            return Err(PATH);
        }
        #[cfg(windows)]
        pins.extend(super::edit_commit::pin_directory(&project, &current).map_err(|_| PATH)?);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            pins.push(
                File::options()
                    .read(true)
                    .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open(&current)
                    .map_err(|_| PATH)?,
            );
        }
    }
    Ok(Some((current, pins)))
}

fn writer(directory: &Path) -> Result<File, Failure> {
    let path = directory.join("writer.lock");
    if let Ok(meta) = fs::symlink_metadata(&path) {
        if linked(&meta) || !meta.is_file() {
            return Err(PATH);
        }
    }
    let file = file_options(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|_| BUSY)?;
    inspect(&file)?;
    if file.metadata().map_err(|_| PATH)?.len() != 0 {
        return Err(PATH);
    }
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: flock borrows this live descriptor. Closing it releases the lock.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(BUSY);
        }
    }
    Ok(file)
}

impl Store {
    #[cfg(test)]
    pub(super) fn fail_writes(&self) {
        self.db.execute_batch("PRAGMA query_only=ON;").unwrap();
    }

    pub(super) fn open_existing(root: &Path) -> Result<Option<Self>, Failure> {
        let Some((directory, pins)) = directory(root, false)? else {
            return Ok(None);
        };
        let path = directory.join("jobs.db");
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(PATH),
            Ok(meta) if linked(&meta) || !meta.is_file() => return Err(PATH),
            _ => {}
        }
        Self::open(directory, pins, false).map(Some)
    }
    pub(super) fn create(root: &Path) -> Result<Self, Failure> {
        let (directory, pins) = directory(root, true)?.ok_or(IO)?;
        Self::open(directory, pins, true)
    }
    fn open(directory: PathBuf, pins: Vec<File>, create: bool) -> Result<Self, Failure> {
        let writer = writer(&directory)?;
        if create {
            let ignore = directory.join(".gitignore");
            match file_options(false).create_new(true).open(&ignore) {
                Ok(mut file) => {
                    file.write_all(b"*\n").map_err(|_| IO)?;
                    file.sync_all().map_err(|_| IO)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let mut file = checked_file(&ignore, false)?;
                    let mut content = String::new();
                    Read::by_ref(&mut file)
                        .take(5)
                        .read_to_string(&mut content)
                        .map_err(|_| PATH)?;
                    if content != "*\n" && content != "*\r\n" {
                        return Err(PATH);
                    }
                }
                Err(_) => return Err(IO),
            }
        }
        let path = directory.join("jobs.db");
        let pin = if create {
            let file = file_options(false)
                .create_new(true)
                .open(&path)
                .map_err(|_| FORMAT)?;
            inspect(&file)?;
            file
        } else {
            checked_file(&path, false)?
        };
        let files = vec![pin];
        // Inspect known sidecars before SQLite can recover or open the database.
        // Unlike the database itself, sidecars permit SQLite recovery deletion.
        for suffix in ["-journal", "-wal", "-shm"] {
            drop(sidecar_file(&directory.join(format!("jobs.db{suffix}")))?);
        }
        let db = sql(Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        ))?;
        sql(db.busy_timeout(Duration::from_millis(100)))?;
        sql(db.set_limit(
            rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,
            MAX_SNAPSHOT as i32,
        ))?;
        sql(db.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_SQL_LENGTH, 16384))?;
        sql(db.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA synchronous=FULL; PRAGMA temp_store=MEMORY; PRAGMA cache_size=-2048;"))?;
        if create {
            sql(db.execute_batch("PRAGMA journal_mode=PERSIST; PRAGMA max_page_count=16384; BEGIN IMMEDIATE; CREATE TABLE tasks(position INTEGER PRIMARY KEY, data TEXT NOT NULL); CREATE TABLE events(cursor INTEGER PRIMARY KEY, data TEXT NOT NULL); CREATE TABLE state(id INTEGER PRIMARY KEY CHECK(id=1), cursor INTEGER NOT NULL); INSERT INTO state VALUES(1,0); PRAGMA application_id=1212821314; PRAGMA user_version=1; COMMIT;"))?;
        } else {
            let version: i64 = db
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .map_err(|_| FORMAT)?;
            let application: i64 = db
                .pragma_query_value(None, "application_id", |row| row.get(0))
                .map_err(|_| FORMAT)?;
            if version != 1 || application != 1212821314 {
                return Err(FORMAT);
            }
            let objects: i64 = db
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| FORMAT)?;
            if objects != 3 {
                return Err(FORMAT);
            }
            let mode: String = db
                .pragma_query_value(None, "journal_mode", |row| row.get(0))
                .map_err(|_| FORMAT)?;
            if mode == "wal" {
                return Err(FORMAT);
            }
            let check: String = db
                .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
                .map_err(|_| FORMAT)?;
            if check != "ok" {
                return Err(FORMAT);
            }
            sql(db.execute_batch("PRAGMA journal_mode=PERSIST; PRAGMA max_page_count=16384;"))?;
        }
        let store = Self {
            db,
            _files: files,
            _writer: writer,
            _directories: pins,
            directory,
        };
        store.load()?;
        Ok(store)
    }
    pub(super) fn load(&self) -> Result<Value, Failure> {
        let cursor: i64 = self
            .db
            .query_row("SELECT cursor FROM state WHERE id=1", [], |row| row.get(0))
            .map_err(|_| FORMAT)?;
        if cursor < 0 {
            return Err(FORMAT);
        }
        let mut tasks: Vec<Value> = Vec::new();
        let mut events: Vec<Value> = Vec::new();
        let mut bytes = 0usize;
        for (query, target, maximum) in [
            ("SELECT data FROM tasks ORDER BY position", &mut tasks, 64),
            ("SELECT data FROM events ORDER BY cursor", &mut events, 256),
        ] {
            let mut statement = self.db.prepare(query).map_err(|_| FORMAT)?;
            let mut rows = statement.query([]).map_err(|_| FORMAT)?;
            while let Some(row) = rows.next().map_err(|_| FORMAT)? {
                let text: String = row.get(0).map_err(|_| FORMAT)?;
                bytes = bytes.checked_add(text.len()).ok_or(FORMAT)?;
                if target.len() >= maximum || bytes > MAX_SNAPSHOT {
                    return Err(FORMAT);
                }
                target.push(serde_json::from_str(&text).map_err(|_| FORMAT)?);
            }
        }
        let snapshot = json!({"tasks":tasks,"events":events,"cursor":cursor});
        validate(&snapshot)?;
        Ok(snapshot)
    }
    pub(super) fn save(&self, snapshot: &Value) -> Result<(), Failure> {
        validate(snapshot)?;
        inspect(&self._files[0])?;
        // Recovery may have deleted and recreated a sidecar. Inspect current
        // names and close these handles before SQLite may delete/recreate them.
        for suffix in ["-journal", "-wal", "-shm"] {
            drop(sidecar_file(
                &self.directory.join(format!("jobs.db{suffix}")),
            )?);
        }
        let tasks = snapshot["tasks"].as_array().ok_or(FORMAT)?;
        let events = snapshot["events"].as_array().ok_or(FORMAT)?;
        let tx = sql(rusqlite::Transaction::new_unchecked(
            &self.db,
            TransactionBehavior::Immediate,
        ))?;
        for (position, task) in tasks.iter().enumerate() {
            sql(tx.execute("INSERT INTO tasks(position,data) VALUES(?1,?2) ON CONFLICT(position) DO UPDATE SET data=excluded.data WHERE tasks.data<>excluded.data", params![position as i64, task.to_string()]))?;
        }
        sql(tx.execute("DELETE FROM tasks WHERE position>=?1", [tasks.len() as i64]))?;
        for event in events {
            sql(tx.execute("INSERT INTO events(cursor,data) VALUES(?1,?2) ON CONFLICT(cursor) DO UPDATE SET data=excluded.data WHERE events.data<>excluded.data", params![event["cursor"].as_u64().ok_or(FORMAT)? as i64, event.to_string()]))?;
        }
        if let Some(first) = events.first() {
            sql(tx.execute(
                "DELETE FROM events WHERE cursor<?1",
                [first["cursor"].as_u64().ok_or(FORMAT)? as i64],
            ))?;
        } else {
            sql(tx.execute("DELETE FROM events", []))?;
        }
        sql(tx.execute(
            "UPDATE state SET cursor=?1 WHERE id=1",
            [snapshot["cursor"].as_u64().ok_or(FORMAT)? as i64],
        ))?;
        sql(tx.commit())?;
        Ok(())
    }
}

fn validate(snapshot: &Value) -> Result<(), Failure> {
    let object = snapshot.as_object().ok_or(FORMAT)?;
    if object.len() != 3 {
        return Err(FORMAT);
    }
    let tasks = snapshot["tasks"].as_array().ok_or(FORMAT)?;
    let events = snapshot["events"].as_array().ok_or(FORMAT)?;
    let cursor = snapshot["cursor"]
        .as_u64()
        .filter(|value| *value < i64::MAX as u64)
        .ok_or(FORMAT)?;
    if tasks.len() > 64 || events.len() > 256 || snapshot.to_string().len() > MAX_SNAPSHOT {
        return Err(FORMAT);
    }
    if tasks
        .iter()
        .any(|value| !value.is_object() || value.to_string().len() > 128 * 1024)
    {
        return Err(FORMAT);
    }
    let mut previous = 0;
    for event in events {
        let sequence = event["cursor"].as_u64().ok_or(FORMAT)?;
        if sequence <= previous || sequence > cursor || event.to_string().len() > 128 * 1024 {
            return Err(FORMAT);
        }
        previous = sequence;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn crash_snapshot() -> Value {
        json!({"tasks":(0..64).map(|index| json!({"taskId":index,"padding":"x".repeat(100 * 1024)})).collect::<Vec<_>>(),"events":[],"cursor":1})
    }
    #[test]
    fn hot_journal_fixture() {
        let Some(root) = std::env::var_os("HARNESS_TASK_STORE_CRASH_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let store = Store::create(&root).unwrap();
        store.save(&crash_snapshot()).unwrap();
        let path = root.join(".agents/harness/jobs/jobs.db");
        let before = harness_domain::hash(&fs::read(&path).unwrap());
        store
            .db
            .execute_batch("PRAGMA cache_size=10; BEGIN IMMEDIATE;")
            .unwrap();
        store
            .db
            .execute(
                "UPDATE tasks SET data=?1",
                [json!({"padding":"y".repeat(100 * 1024)}).to_string()],
            )
            .unwrap();
        assert!(
            fs::metadata(root.join(".agents/harness/jobs/jobs.db-journal"))
                .unwrap()
                .len()
                > 0
        );
        assert_ne!(
            before,
            harness_domain::hash(&fs::read(path).unwrap()),
            "dirty pages must spill before the crash"
        );
        // Deliberately bypass Store/SQLite destructors, leaving an actual hot journal.
        std::process::exit(0);
    }
    #[test]
    fn hot_journal_recovers_after_process_crash() {
        let root = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "adapters::tasks_store::tests::hot_journal_fixture",
                "--nocapture",
            ])
            .env("HARNESS_TASK_STORE_CRASH_ROOT", root.path())
            .status()
            .unwrap();
        assert!(status.success());
        let store = Store::open_existing(root.path()).unwrap().unwrap();
        assert_eq!(store.load().unwrap(), crash_snapshot());
        store.save(&snapshot(2)).unwrap();
        assert_eq!(store.load().unwrap(), snapshot(2));
    }
    fn snapshot(cursor: u64) -> Value {
        json!({"tasks":[{"id":"fixture","status":"succeeded"}],"events":[{"cursor":cursor,"kind":"state"}],"cursor":cursor})
    }
    #[test]
    fn lazy_storage_reopens_and_excludes_second_writer() {
        let root = tempfile::tempdir().unwrap();
        assert!(Store::open_existing(root.path()).unwrap().is_none());
        assert!(!root.path().join(".agents").exists());
        let store = Store::create(root.path()).unwrap();
        assert_eq!(
            fs::read(root.path().join(".agents/harness/jobs/.gitignore")).unwrap(),
            b"*\n"
        );
        store.save(&snapshot(1)).unwrap();
        assert!(matches!(
            Store::open_existing(root.path()),
            Err(("TASK_STORE_BUSY", _))
        ));
        drop(store);
        let store = Store::open_existing(root.path()).unwrap().unwrap();
        assert_eq!(store.load().unwrap(), snapshot(1));
    }
    #[test]
    fn failed_transaction_preserves_last_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::create(root.path()).unwrap();
        store.save(&snapshot(1)).unwrap();
        store.db.execute_batch("CREATE TEMP TRIGGER reject_state BEFORE UPDATE ON state BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert!(store.save(&snapshot(2)).is_err());
        assert_eq!(store.load().unwrap(), snapshot(1));
        assert!(
            store
                .save(&json!({"tasks":[],"events":[],"cursor":u64::MAX}))
                .is_err()
        );
    }
    #[test]
    fn corrupt_database_is_not_reinitialized() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::create(root.path()).unwrap();
        drop(store);
        let path = root.path().join(".agents/harness/jobs/jobs.db");
        fs::write(&path, b"fixture corrupt db").unwrap();
        assert!(Store::open_existing(root.path()).is_err());
        assert_eq!(fs::read(path).unwrap(), b"fixture corrupt db");
    }
    #[test]
    fn rejects_hard_linked_database_and_lock() {
        for name in [
            "jobs.db",
            "writer.lock",
            "jobs.db-journal",
            "jobs.db-wal",
            "jobs.db-shm",
        ] {
            let root = tempfile::tempdir().unwrap();
            let store = Store::create(root.path()).unwrap();
            drop(store);
            let path = root.path().join(".agents/harness/jobs").join(name);
            fs::hard_link(&path, root.path().join("linked")).unwrap();
            assert!(Store::open_existing(root.path()).is_err());
        }
    }
}

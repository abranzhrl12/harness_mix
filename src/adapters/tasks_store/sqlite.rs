use super::{
    FORMAT, Failure, IO, MAX_SNAPSHOT, PATH, Store,
    filesystem::{checked_file, directory, file_options, inspect, linked, sidecar_file, writer},
    snapshot::validate,
};
use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};
use serde_json::{Value, json};
use std::{
    fs,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

impl Store {
    #[cfg(test)]
    pub(in crate::adapters) fn fail_writes(&self) {
        self.db.execute_batch("PRAGMA query_only=ON;").unwrap();
    }

    pub(in crate::adapters) fn open_existing(root: &Path) -> Result<Option<Self>, Failure> {
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
    pub(in crate::adapters) fn create(root: &Path) -> Result<Self, Failure> {
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
    pub(in crate::adapters) fn load(&self) -> Result<Value, Failure> {
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
    pub(in crate::adapters) fn save(&self, snapshot: &Value) -> Result<(), Failure> {
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

fn sql<T>(result: rusqlite::Result<T>) -> Result<T, Failure> {
    result.map_err(|_| IO)
}

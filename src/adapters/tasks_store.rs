//! Fixed-path, single-writer SQLite journal for managed verification jobs.
use harness_domain::Failure;
use rusqlite::Connection;
use std::path::PathBuf;

mod filesystem;
mod snapshot;
mod sqlite;
#[cfg(test)]
mod tests;

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
    _files: Vec<std::fs::File>,
    _writer: std::fs::File,
    _directories: Vec<std::fs::File>,
    directory: PathBuf,
}

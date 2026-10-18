//! Explicit empty-index bootstrap. Never migrate or overwrite an existing index.
use super::*;

/// Create the schema only; source extraction remains an explicit sync operation.
/// An interrupted attempt is retained, not silently reused or recursively deleted.
pub fn initialize(root: &Path, context: &OperationContext) -> Result<Value, Failure> {
    context.check()?;
    #[cfg(windows)]
    {
        initialize_windows(root, context)
    }
    #[cfg(not(windows))]
    {
        let _ = root;
        Err((
            "SYNC_PLATFORM",
            "Native index initialization currently requires Windows",
        ))
    }
}

#[cfg(windows)]
fn initialize_windows(root: &Path, context: &OperationContext) -> Result<Value, Failure> {
    use std::{fs::File, os::windows::fs::OpenOptionsExt};
    let project = Project::new(root).map_err(|_| ("PATH", "Workspace unavailable"))?;
    let _root_pins = super::super::edit_commit::pin_directory(&project, &project.root)?;
    let directory = project.target_optional(".codegraph")?;
    // create_dir (not create_dir_all) is our exclusive claim. Refuse even an
    // empty existing directory, dangling link, old index, or interrupted init.
    std::fs::create_dir(&directory).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            (
                "INIT_EXISTS",
                "Index directory already exists; initialization never overwrites it",
            )
        } else {
            ("INIT_IO", "Cannot create index directory")
        }
    })?;
    let initialize = || {
        let _pins = super::super::edit_commit::pin_directory(&project, &directory)?;
        let path = project.target_optional(".codegraph/codegraph.db")?;
        let mut guards = Vec::new();
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let candidate = project.target_optional(&format!(".codegraph/codegraph.db{suffix}"))?;
            guards.push(
                File::options()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .share_mode(3)
                    .custom_flags(0x0020_0000)
                    .open(candidate)
                    .map_err(|_| INDEX)?,
            );
        }
        context.check()?;
        let mut db = sql(Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        ))?;
        sql(db.busy_timeout(Duration::from_millis(100)))?;
        // This database is NEW and has no committed user data. OFF avoids a
        // rollback-journal unlink while our deterministic sidecars are pinned.
        // Failure leaves an unusable namespace for manual inspection, never an
        // apparently valid partial schema: version 11 is written last.
        sql(db.execute_batch(
            "PRAGMA journal_mode=OFF; PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;",
        ))?;
        let cancel = context.clone();
        sql(db.progress_handler(1000, Some(move || cancel.check().is_err())))?;
        let tx = sql(db.transaction())?;
        sql(tx.execute_batch(include_str!("../../vendor/codegraph-native/schema.sql")))?;
        sql(tx.execute("INSERT INTO project_metadata(key,value,updated_at) VALUES('harness.native.profile','empty-index-v1',0)", []))?;
        context.check()?;
        sql(tx.execute("INSERT INTO schema_versions(version,applied_at,description) VALUES(11,0,'Native fresh schema; not an upstream migration')", []))?;
        validate_schema(&tx)?;
        tx.progress_handler(0, None::<fn() -> bool>)
            .map_err(|_| INDEX)?;
        sql(tx.commit())?;
        let journal: String = sql(db.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0)))?;
        if journal != "wal" {
            return Err(INDEX);
        }
        db.close().map_err(|_| INDEX)?;
        guards[0].sync_all().map_err(|_| INDEX)?;
        Ok(
            json!({"initialized":true,"schemaVersion":11,"journalMode":"wal",
            "indexPath":".codegraph/codegraph.db","filesIndexed":0,
            "synchronized":false,"globalFreshnessCertified":false,
            "nextStep":"Run --sync; initialization creates an empty index only"}),
        )
    };
    initialize().map_err(|_| ("INIT_INCOMPLETE",
        "Initialization did not complete; retained .codegraph for inspection; no existing index was overwritten"))
}

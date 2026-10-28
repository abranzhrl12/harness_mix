use super::*;
const OWNER: &str = ".codegraph/.harness-owner";
const FILES: &[&str] = &[
    "codegraph.db",
    "codegraph.db-wal",
    "codegraph.db-shm",
    "codegraph.db-journal",
    "index-write.lock",
    "autosync.lock",
    "autosync.status",
    ".harness-owner",
];

pub(super) fn claim(root: &Path, id: &str) -> Result<(), String> {
    fs::create_dir(root.join(".codegraph"))
        .map_err(|e| format!("cannot claim new index directory: {e}"))?;
    storage::write_safe_file(root, Path::new(OWNER), id.as_bytes(), None)
}
pub(super) fn verify_owner(root: &Path, id: &str) -> Result<(), String> {
    if storage::read_safe_file(root, Path::new(OWNER))?.as_deref() != Some(id.as_bytes()) {
        return Err("index ownership marker missing or changed; index preserved".into());
    }
    Ok(())
}
pub(super) fn preflight_remove(root: &Path, id: &str) -> Result<(), String> {
    if storage::safe_metadata(root, Path::new(".codegraph"))?.is_none() {
        return Ok(());
    }
    verify_owner(root, id)?;
    for entry in fs::read_dir(root.join(".codegraph")).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        if !FILES.iter().any(|allowed| name == *allowed) {
            return Err("unrecognized files in owned index; directory preserved".into());
        }
        let relative = Path::new(".codegraph").join(&name);
        if !storage::safe_metadata(root, &relative)?.is_some_and(|m| m.is_file()) {
            return Err("unsafe index entry; preserved".into());
        }
        if name == "autosync.status" {
            let contents = storage::read_safe_file(root, &relative)?
                .ok_or("watcher status disappeared; index preserved")?;
            if !contents.starts_with(b"harness-watch-status-v1\n") {
                return Err("unrecognized watcher status; index preserved".into());
            }
        }
        if name == "autosync.lock" && entry.metadata().map_err(|e| e.to_string())?.len() != 0 {
            return Err("unrecognized watcher lock; index preserved".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(0)
                .open(root.join(&relative))
                .map_err(
                    |_| "index in use; close the project's MCP connections before uninstalling",
                )?;
        }
    }
    Ok(())
}
pub(super) fn remove(root: &Path, id: &str) -> Result<(), String> {
    if storage::safe_metadata(root, Path::new(".codegraph"))?.is_none() {
        return Ok(());
    }
    preflight_remove(root, id)?;
    // Explicit allowlist, no recursive delete. Keep marker until all DB files
    // have gone so an interrupted uninstall can safely resume.
    for name in FILES.iter().filter(|name| **name != ".harness-owner") {
        let relative = Path::new(".codegraph").join(name);
        if storage::safe_metadata(root, &relative)?.is_some() {
            fs::remove_file(root.join(relative))
                .map_err(|e| format!("index file could not be removed: {e}"))?;
        }
    }
    storage::remove_safe_file(root, Path::new(OWNER), &hash(id.as_bytes()))?;
    fs::remove_dir(root.join(".codegraph")).map_err(|e| format!("index directory preserved: {e}"))
}

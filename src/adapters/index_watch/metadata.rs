//! Advisory watcher telemetry. The OS watcher lease, never this file, elects an owner.
#[cfg(test)]
#[path = "metadata_tests.rs"]
mod tests;
use super::super::project::Project;
use serde_json::{Value, json};
use std::{fs::File, io::{Read, Seek, SeekFrom, Write}, path::Path};
use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
use windows::Win32::{Foundation::HANDLE, Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle}};

const MAGIC: &[u8] = b"harness-watch-status-v1\n";
const LIMIT: u64 = 8192;

fn open(root: &Path, write: bool) -> Result<File, ()> {
    let project = Project::new(root).map_err(|_| ())?;
    let path = project.target_optional(".codegraph/autosync.status").map_err(|_| ())?;
    let file = File::options().read(true).write(write).create(write)
        .truncate(false).share_mode(3).custom_flags(0x0020_0000)
        .open(path).map_err(|_| ())?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }.map_err(|_| ())?;
    let metadata = file.metadata().map_err(|_| ())?;
    if info.nNumberOfLinks != 1 || info.dwFileAttributes & 0x400 != 0
        || !metadata.is_file() || metadata.len() > LIMIT { return Err(()); }
    Ok(file)
}

fn read_record(file: &mut File) -> Result<Option<Value>, ()> {
    file.seek(SeekFrom::Start(0)).map_err(|_| ())?;
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes).map_err(|_| ())?;
    if bytes.is_empty() { return Ok(None); }
    if bytes.len() as u64 > LIMIT || !bytes.starts_with(MAGIC) { return Err(()); }
    serde_json::from_slice(&bytes[MAGIC.len()..]).map(Some).map_err(|_| ())
}

pub(super) fn publish(root: &Path, status: &Value, now: u64) -> Result<(), ()> {
    let mut file = open(root, true)?;
    file.try_lock().map_err(|_| ())?;
    // A recognized but interrupted record can be replaced. Never overwrite
    // unrelated contents, even if a stale file happens to have this name.
    file.seek(SeekFrom::Start(0)).map_err(|_| ())?;
    let mut prefix = vec![0; MAGIC.len()];
    let length = file.read(&mut prefix).map_err(|_| ())?;
    if length != 0 && (length != MAGIC.len() || prefix != MAGIC) { return Err(()); }
    let record = json!({
        "pid": status["ownerPid"], "startedAtUnixSeconds": status["ownerStartedAtUnixSeconds"],
        "heartbeatUnixSeconds": now, "state": status["state"],
        "lastSuccessfulSyncUnixSeconds": status["lastSuccessfulSyncUnixSeconds"],
        "lastError": status["lastError"]
    });
    let bytes = serde_json::to_vec(&record).map_err(|_| ())?;
    if bytes.len() as u64 + MAGIC.len() as u64 > LIMIT { return Err(()); }
    file.seek(SeekFrom::Start(0)).map_err(|_| ())?;
    file.write_all(MAGIC).and_then(|_| file.write_all(&bytes)).map_err(|_| ())?;
    file.set_len((MAGIC.len() + bytes.len()) as u64).map_err(|_| ())?;
    file.flush().map_err(|_| ())
}

pub(super) fn observe(root: &Path) -> Option<Value> {
    let mut file = open(root, false).ok()?;
    file.try_lock_shared().ok()?;
    read_record(&mut file).ok().flatten()
}

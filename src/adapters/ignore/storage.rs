use super::policy;
use harness_domain::Failure;
use std::{fs, io::Write, path::Path};

pub(super) fn read_policy(path: &Path) -> Result<Option<(String, usize, usize)>, Failure> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(("IO", "Cannot inspect .harnessignore")),
    };
    if !metadata.is_file() || metadata.len() as usize > policy::MAX_FILE_BYTES {
        return Err((
            "PATH",
            ".harnessignore must be a regular file of at most 256 KiB",
        ));
    }
    let bytes = fs::read(path).map_err(|_| ("IO", "Cannot read .harnessignore"))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| ("ENCODING", ".harnessignore must be UTF-8"))?
        .trim_start_matches('\u{feff}')
        .to_owned();
    let (entries, patterns) = policy::entries_of(&text);
    if patterns > super::MAX_PATTERNS {
        return Err((
            "LIMIT",
            "Existing .harnessignore exceeds 128 patterns; reduce it before managing it",
        ));
    }
    Ok(Some((text, patterns, entries)))
}

pub(super) fn write_atomic(
    path: &Path,
    content: &str,
    replace_existing: bool,
) -> Result<(), Failure> {
    let parent = path.parent().ok_or(("PATH", "Missing ignore parent"))?;
    let mut stage = tempfile::Builder::new()
        .prefix(".harness-ignore-")
        .tempfile_in(parent)
        .map_err(|_| ("IO", "Cannot create ignore staging file"))?;
    stage
        .write_all(content.as_bytes())
        .map_err(|_| ("IO", "Cannot stage ignore policy"))?;
    stage
        .as_file()
        .sync_all()
        .map_err(|_| ("IO", "Cannot sync ignore policy"))?;
    if replace_existing {
        replace_staged(stage, path)
    } else {
        stage.persist_noclobber(path).map(|_| ()).map_err(|error| {
            if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                (
                    "STALE",
                    "Ignore policy appeared concurrently; reread and retry",
                )
            } else {
                ("IO", "Cannot publish ignore policy")
            }
        })
    }
}

#[cfg(windows)]
fn replace_staged(stage: tempfile::NamedTempFile, target: &Path) -> Result<(), Failure> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::Storage::FileSystem::{REPLACE_FILE_FLAGS, ReplaceFileW},
        core::PCWSTR,
    };
    let stage = stage.into_temp_path();
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    let target_w = wide(target);
    let stage_w = wide(&stage);
    // SAFETY: NUL-terminated buffers live for the duration of the call.
    let result = unsafe {
        ReplaceFileW(
            PCWSTR(target_w.as_ptr()),
            PCWSTR(stage_w.as_ptr()),
            PCWSTR::null(),
            REPLACE_FILE_FLAGS(0),
            None,
            None,
        )
    };
    if result.is_err() {
        // ReplaceFileW has no orphan backup here; inspect the target rather than guessing.
        let current = fs::read(target).map_err(|_| {
            (
                "EDIT_OUTCOME_UNKNOWN",
                "Ignore publication failed; inspect .harnessignore before retrying",
            )
        })?;
        let _ = fs::remove_file(&stage);
        if current.is_empty() {
            return Err(("IO", "Cannot publish ignore policy"));
        }
        return Err((
            "EDIT_OUTCOME_UNKNOWN",
            "Ignore publication failed; reread .harnessignore before retrying",
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_staged(stage: tempfile::NamedTempFile, target: &Path) -> Result<(), Failure> {
    stage
        .persist(target)
        .map(|_| ())
        .map_err(|_| ("IO", "Cannot publish ignore policy"))
}

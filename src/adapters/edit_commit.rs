//! Windows-only publication with pinned ancestors, recheck and ACL-preserving replacement.
use super::project::Project;
use harness_domain::{Failure, contracts::ReplaceArgs, execution::OperationContext, hash};
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

pub(super) fn target(project: &Project, relative: &str) -> Result<PathBuf, Failure> {
    let target = project.target(relative)?;
    let normalized = target
        .strip_prefix(&project.root)
        .map_err(|_| ("PATH", "Outside workspace"))?
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    let knowledge_document = is_agent_knowledge_document(&normalized);
    if normalized
        .split('/')
        .any(|part| [".git", ".hg", ".svn"].contains(&part))
        || (normalized == ".agents" || normalized.starts_with(".agents/")) && !knowledge_document
        || normalized == "agents.md"
    {
        return Err((
            "PATH",
            "Protected harness, instruction or version control path",
        ));
    }
    let meta = fs::metadata(&target).map_err(|_| ("PATH", "File unavailable"))?;
    if !meta.is_file() || meta.permissions().readonly() {
        return Err(("PATH", "Not a writable regular file"));
    }
    Ok(target)
}

/// Only existing Markdown knowledge documents are editable under `.agents`.
/// Agent instructions, runtime state, credentials, and other metadata stay protected.
pub(super) fn is_agent_knowledge_document(relative: &str) -> bool {
    let normalized = relative.replace('\\', "/").to_ascii_lowercase();
    let parts: Vec<_> = normalized.split('/').collect();
    parts.len() >= 3
        && parts[0] == ".agents"
        && parts[1] == "knowledge"
        && parts[2..]
            .iter()
            .all(|part| !part.is_empty() && *part != "." && *part != "..")
        && !parts[2..]
            .iter()
            .any(|part| [".git", ".hg", ".svn", ".agents"].contains(part))
        && parts
            .last()
            .is_some_and(|name| *name != "agents.md" && name.ends_with(".md"))
}

#[cfg(windows)]
pub(super) fn pin_parents(project: &Project, target: &Path) -> Result<Vec<File>, Failure> {
    let pins = pin_directory(project, target.parent().ok_or(("PATH", "Missing parent"))?)?;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle},
    };
    let file = File::open(target).map_err(|_| ("PATH", "File unavailable"))?;
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut information)
            .map_err(|_| ("IO", "Cannot inspect file identity"))?;
    }
    if information.nNumberOfLinks != 1 {
        return Err(("PATH", "Hard-linked files cannot be edited"));
    }
    Ok(pins)
}

#[cfg(windows)]
pub(super) fn pin_directory(project: &Project, parent: &Path) -> Result<Vec<File>, Failure> {
    parent
        .strip_prefix(&project.root)
        .map_err(|_| ("PATH", "Outside workspace"))?;
    parent
        .ancestors()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(pin_one_directory)
        .collect()
}

/// Hold a directory against rename/deletion, without following reparse points.
#[cfg(windows)]
pub(super) fn pin_one_directory(path: &Path) -> Result<File, Failure> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    let file = File::options()
        .read(true)
        .custom_flags(0x0200_0000 | 0x0020_0000)
        .share_mode(3)
        .open(path)
        .map_err(|_| ("EDIT_BUSY", "Cannot pin ancestor against rename"))?;
    let meta = file
        .metadata()
        .map_err(|_| ("IO", "Directory metadata unavailable"))?;
    if !meta.is_dir() || meta.file_attributes() & 0x400 != 0 {
        return Err(("PATH", "Reparse or non-directory ancestor"));
    }
    Ok(file)
}

#[cfg(not(windows))]
pub(super) fn pin_parents(_: &Project, _: &Path) -> Result<Vec<File>, Failure> {
    Err((
        "UNSUPPORTED",
        "Editing requires Windows ancestor pinning in this release",
    ))
}

pub(super) fn publish(
    project: &Project,
    args: &ReplaceArgs,
    target_path: &Path,
    updated: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    publish_with(project, args, target_path, updated, context, || {})
}

fn publish_with(
    project: &Project,
    args: &ReplaceArgs,
    target_path: &Path,
    updated: &str,
    context: &OperationContext,
    before_recheck: impl FnOnce(),
) -> Result<Value, Failure> {
    let parent = target_path.parent().ok_or(("PATH", "Missing parent"))?;
    let mut stage = tempfile::Builder::new()
        .prefix(".harness-edit-")
        .tempfile_in(parent)
        .map_err(|_| ("IO", "Cannot create edit staging file"))?;
    super::edit_security::protect_stage(target_path, stage.path())?;
    stage
        .write_all(updated.as_bytes())
        .map_err(|_| ("IO", "Cannot stage edit"))?;
    stage
        .as_file()
        .sync_all()
        .map_err(|_| ("IO", "Cannot sync edit"))?;
    before_recheck();
    context.check()?;
    target(project, &args.path)?;
    if hash(&project.bytes_with_context(&args.path, context)?) != args.hash {
        return Err(("STALE", "Concurrent change detected before publication"));
    }
    context.check()?;
    // Commit point: do not report a later cancellation as if the edit never happened.
    publish_staged(stage, target_path, project, &args.hash)
}

#[cfg(windows)]
fn publish_staged(
    stage: tempfile::NamedTempFile,
    target: &Path,
    project: &Project,
    base_hash: &str,
) -> Result<Value, Failure> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::Storage::FileSystem::{REPLACE_FILE_FLAGS, ReplaceFileW},
        core::PCWSTR,
    };
    let parent = target.parent().ok_or(("PATH", "Missing parent"))?;
    let backup = parent.join(format!(".harness-recovery-{}", uuid::Uuid::new_v4()));
    if backup.exists() {
        return Err(("IO", "Recovery path collision"));
    }
    // Close the staging handle, retaining ownership of the temporary path.
    let stage = stage.into_temp_path();
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    let target_w = wide(target);
    let stage_w = wide(&stage);
    let backup_w = wide(&backup);
    // SAFETY: all UTF-16 buffers are terminated and live through this synchronous call.
    // Flags intentionally do not ignore ACL/attribute merge failures.
    let result = unsafe {
        ReplaceFileW(
            PCWSTR(target_w.as_ptr()),
            PCWSTR(stage_w.as_ptr()),
            PCWSTR(backup_w.as_ptr()),
            REPLACE_FILE_FLAGS(0),
            None,
            None,
        )
    };
    let relative = |path: &Path| {
        path.strip_prefix(&project.root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string()
    };
    if result.is_err() {
        // ReplaceFileW may have progressed partially; the target content decides the outcome.
        let staged_path = stage.keep().map_err(|_| {
            (
                "EDIT_OUTCOME_UNKNOWN",
                "Publication failed; inspect .harness recovery files before retrying",
            )
        })?;
        let (target_hash, unchanged) = inspect_target_after_failure(target, base_hash);
        if unchanged {
            // Known failure: the target is byte-identical to the pre-edit hash, so no edit
            // was applied. Remove the orphan candidate and backup automatically.
            let candidate_removed = fs::remove_file(&staged_path).is_ok();
            let backup_removed = fs::remove_file(&backup).is_ok();
            return Ok(json!({"ok":false,"changed":false,"commitStatus":"unchanged",
                "targetHash":target_hash,
                "recovery":{"candidate":Value::Null,"backup":Value::Null,
                    "candidateRemoved":candidate_removed,"backupRemoved":backup_removed},
                "diagnostics":[{"code":"EDIT_OUTCOME_UNKNOWN",
                    "message":"Publication failed but the target is unchanged; staging artifacts were removed",
                    "retryable":false,"requiredAction":"retry_edit_with_a_fresh_hash"}]}));
        }
        // Truly unknown outcome: preserve both recovery artifacts and report the post-failure hash.
        return Ok(json!({"ok":false,"changed":Value::Null,"commitStatus":"unknown",
            "targetHash":target_hash,
            "recovery":{"backup":relative(&backup),"candidate":relative(&staged_path)},
            "diagnostics":[{"code":"EDIT_OUTCOME_UNKNOWN",
                "message":"Publication failed; inspect target and recovery artifacts before retrying",
                "retryable":false,"requiredAction":"reread_target_then_retry"}]}));
    }
    let warning = if fs::remove_file(&backup).is_err() {
        Some(relative(&backup))
    } else {
        None
    };
    Ok(
        json!({"path":relative(target),"changed":true,"commitStatus":"committed","recoveryBackup":warning}),
    )
}

#[cfg(not(windows))]
fn publish_staged(
    _: tempfile::NamedTempFile,
    _: &Path,
    _: &Project,
    _: &str,
) -> Result<Value, Failure> {
    Err((
        "UNSUPPORTED",
        "Editing publication is not implemented on this platform",
    ))
}

/// Post-failure target state: `(current hash, unchanged)`.
/// `None` means the target could not be read, so the outcome stays unknown.
#[cfg(windows)]
fn inspect_target_after_failure(target: &Path, base_hash: &str) -> (Option<String>, bool) {
    match fs::read(target) {
        Ok(bytes) => {
            let current = hash(&bytes);
            let unchanged = current == base_hash;
            (Some(current), unchanged)
        }
        Err(_) => (None, false),
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn staged_edit_rechecks_cancellation_and_hash_and_cleans_temporaries() {
        for cancel in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let file = root.path().join("file.txt");
            fs::write(&file, "old").unwrap();
            let project = Project::new(root.path()).unwrap();
            let context = OperationContext::default();
            let args = ReplaceArgs {
                bom: None,
                path: "file.txt".into(),
                hash: hash(b"old"),
                chunks: None,
                old_text: "old".into(),
                new_text: "new".into(),
                allow_unvalidated_text: false,
                preview: false,
                session: None,
            };
            let _pins = pin_parents(&project, &project.target("file.txt").unwrap()).unwrap();
            let result = publish_with(&project, &args, &file, "new", &context, || {
                if cancel {
                    context.cancel();
                } else {
                    fs::write(&file, "external").unwrap();
                }
            });
            assert_eq!(
                result.unwrap_err().0,
                if cancel { "CANCELLED" } else { "STALE" }
            );
            assert_eq!(
                fs::read_to_string(&file).unwrap(),
                if cancel { "old" } else { "external" }
            );
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn inspect_target_after_failure_classifies_unchanged_changed_and_missing() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("file.txt");
        fs::write(&file, "old").unwrap();
        let (current, unchanged) = inspect_target_after_failure(&file, &hash(b"old"));
        assert_eq!(current.as_deref(), Some(hash(b"old").as_str()));
        assert!(unchanged);
        let (current, unchanged) = inspect_target_after_failure(&file, &hash(b"external"));
        assert_eq!(current.as_deref(), Some(hash(b"old").as_str()));
        assert!(!unchanged);
        let (current, unchanged) =
            inspect_target_after_failure(&root.path().join("missing.txt"), &hash(b"old"));
        assert!(current.is_none());
        assert!(!unchanged);
    }

    #[test]
    fn pinned_ancestors_cannot_be_renamed_during_publication() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("folder")).unwrap();
        let file = root.path().join("folder/file.txt");
        fs::write(&file, "old").unwrap();
        let project = Project::new(root.path()).unwrap();
        let pins = pin_parents(&project, &project.target("folder/file.txt").unwrap()).unwrap();
        assert!(fs::rename(root.path().join("folder"), root.path().join("moved")).is_err());
        drop(pins);
        fs::rename(root.path().join("folder"), root.path().join("moved")).unwrap();
    }
}

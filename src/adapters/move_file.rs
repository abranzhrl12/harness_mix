//! Non-overwriting file moves with pinned Windows paths and explicit post-commit results.
use super::project::Project;
use harness_domain::{Failure, contracts::MoveArgs, execution::OperationContext};
use serde_json::Value;

pub(super) fn move_file(
    project: &Project,
    args: &MoveArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    #[cfg(windows)]
    {
        move_windows(project, args, context, || {})
    }
    #[cfg(not(windows))]
    {
        let _ = (project, args);
        Err((
            "UNSUPPORTED",
            "Moving files requires Windows path pinning in this release",
        ))
    }
}

#[cfg(windows)]
pub(super) fn protected(relative: &str) -> bool {
    relative.split('/').any(|part| {
        [".git", ".hg", ".svn", ".agents", ".codegraph", "agents.md"]
            .iter()
            .any(|name| part.eq_ignore_ascii_case(name))
    })
}

#[cfg(windows)]
fn move_windows(
    project: &Project,
    args: &MoveArgs,
    context: &OperationContext,
    before_commit: impl FnOnce(),
) -> Result<Value, Failure> {
    use super::edit_commit;
    use serde_json::json;
    use std::{
        fs::{self, File},
        os::windows::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawHandle,
        },
        path::Path,
    };
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_RENAME_INFO, FileRenameInfo,
            GetFileInformationByHandle, SetFileInformationByHandle,
        },
    };

    if args.from.len() > 1000 || args.to.len() > 1000 {
        return Err(("PATH", "Move paths exceed 1000 bytes"));
    }
    let source = project.target(&args.from)?;
    let destination = project.target_optional(&args.to)?;
    let relative = |path: &Path| -> Result<String, Failure> {
        Ok(path
            .strip_prefix(&project.root)
            .map_err(|_| ("PATH", "Outside workspace"))?
            .to_string_lossy()
            .replace('\\', "/"))
    };
    // Canonicalize the existing source to use its actual spelling in the index.
    let source = source
        .canonicalize()
        .map_err(|_| ("PATH", "Source unavailable"))?;
    let from = relative(&source)?;
    let to = relative(&destination)?;
    if protected(&from) || protected(&to) || from.is_empty() || to.is_empty() {
        return Err((
            "PATH",
            "Protected harness, instruction, index or version control path",
        ));
    }
    let _source_pins = edit_commit::pin_directory(
        project,
        source.parent().ok_or(("PATH", "Missing source parent"))?,
    )?;
    let _destination_pins = edit_commit::pin_directory(
        project,
        destination
            .parent()
            .ok_or(("PATH", "Destination parent must exist"))?,
    )?;
    // Check again after pinning both ancestor chains. The kernel also refuses an
    // existing destination at commit, including one created after this check.
    project.target(&args.from)?;
    project.target_optional(&args.to)?;
    match fs::symlink_metadata(&destination) {
        Ok(_) => {
            return Err((
                "MOVE_EXISTS",
                "Destination exists; use a free intermediate name for case-only renames",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(("PATH", "Destination unavailable")),
    }
    let file = File::options()
        .access_mode(0x0001_0000 | 0x0080) // DELETE | FILE_READ_ATTRIBUTES
        .share_mode(1) // allow readers, deny competing writes/deletes/renames
        .custom_flags(0x0020_0000) // OPEN_REPARSE_POINT: inspect, never follow
        .open(&source)
        .map_err(|_| {
            (
                "EDIT_BUSY",
                "Cannot acquire exclusive move access to source",
            )
        })?;
    let meta = file
        .metadata()
        .map_err(|_| ("IO", "Cannot inspect move source"))?;
    if !meta.is_file() || meta.permissions().readonly() || meta.file_attributes() & 0x400 != 0 {
        return Err((
            "PATH",
            "Source must be a writable regular file without reparse points",
        ));
    }
    let identity = |file: &File| -> Result<BY_HANDLE_FILE_INFORMATION, Failure> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: the live file owns the handle and info is writable for this call.
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(|_| ("IO", "Cannot inspect file identity"))?;
        Ok(info)
    };
    let before = identity(&file)?;
    if before.nNumberOfLinks != 1 {
        return Err(("PATH", "Hard-linked files cannot be moved"));
    }
    let name: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let length = name.len() * 2;
    let size = std::mem::size_of::<FILE_RENAME_INFO>() + length;
    // Typed storage provides the alignment and enough space for the variable tail.
    let mut buffer =
        vec![FILE_RENAME_INFO::default(); size.div_ceil(std::mem::size_of::<FILE_RENAME_INFO>())];
    let info = buffer.as_mut_ptr();
    // SAFETY: buffer is aligned, zeroed (ReplaceIfExists=false, RootDirectory=NULL),
    // and allocated to include the entire UTF-16 name. No references outlive it.
    unsafe {
        (*info).FileNameLength = (length - 2) as u32;
        let tail = info
            .cast::<u8>()
            .add(std::mem::offset_of!(FILE_RENAME_INFO, FileName))
            .cast::<u16>();
        std::ptr::copy_nonoverlapping(name.as_ptr(), tail, name.len());
    }
    before_commit();
    context.check()?;
    // Commit point. Rename the opened identity, not a pathname that could be swapped.
    // No cross-volume copy fallback and no replacement of an existing destination.
    unsafe {
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileRenameInfo,
            info.cast(),
            size as u32,
        )
    }
    .map_err(|_| {
        (
            "MOVE_IO",
            "Move failed; source retained (destination may exist, be busy or be on another volume)",
        )
    })?;

    // Never turn post-commit cancellation or index failure into a bare move error.
    let verified = File::open(&destination)
        .ok()
        .and_then(|f| identity(&f).ok())
        .is_some_and(|after| {
            before.dwVolumeSerialNumber == after.dwVolumeSerialNumber
                && before.nFileIndexHigh == after.nFileIndexHigh
                && before.nFileIndexLow == after.nFileIndexLow
        })
        && fs::symlink_metadata(&source).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
    drop(file);
    let index = update_index(project, &[&from, &to]);
    let index_failed = index["status"] == "needs_sync";
    let mut result = json!({"from":from,"to":to,"changed":true,"commitStatus":"committed",
        "verified":verified,"importsUpdated":false,"index":index});
    let mut diagnostics = Vec::new();
    if !verified {
        result["ok"] = json!(false);
        diagnostics.push(json!({"code":"MOVE_POSTCHECK","message":"Move completed but postcheck failed; inspect both paths before retrying"}));
    }
    if index_failed {
        diagnostics.push(json!({"code":"MOVE_INDEX","message":"Move completed; index synchronization incomplete. Refresh the index before relying on graph results"}));
    }
    if !diagnostics.is_empty() {
        result["diagnostics"] = json!(diagnostics);
    }
    Ok(result)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn destination_created_after_validation_is_never_overwritten() {
        let root = tempfile::tempdir().unwrap();
        let project = Project::new(root.path()).unwrap();
        fs::write(root.path().join("source.txt"), "original").unwrap();
        let args = MoveArgs {
            from: "source.txt".into(),
            to: "target.txt".into(),
            session: None,
        };
        let error = move_windows(&project, &args, &OperationContext::default(), || {
            fs::write(root.path().join("target.txt"), "concurrent").unwrap();
        })
        .unwrap_err();
        assert_eq!(error.0, "MOVE_IO");
        assert_eq!(
            fs::read_to_string(root.path().join("source.txt")).unwrap(),
            "original"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("target.txt")).unwrap(),
            "concurrent"
        );
    }

    #[test]
    fn source_and_parents_are_pinned_and_cancellation_prevents_commit() {
        let root = tempfile::tempdir().unwrap();
        let project = Project::new(root.path()).unwrap();
        fs::create_dir(root.path().join("dest")).unwrap();
        fs::write(root.path().join("source.txt"), "original").unwrap();
        let args = MoveArgs {
            from: "source.txt".into(),
            to: "dest/target.txt".into(),
            session: None,
        };
        let context = OperationContext::default();
        let error = move_windows(&project, &args, &context, || {
            assert!(
                fs::rename(
                    root.path().join("source.txt"),
                    root.path().join("swapped.txt")
                )
                .is_err()
            );
            assert!(fs::write(root.path().join("source.txt"), "concurrent").is_err());
            assert!(fs::rename(root.path().join("dest"), root.path().join("redirected")).is_err());
            context.cancel();
        })
        .unwrap_err();
        assert_eq!(error.0, "CANCELLED");
        assert_eq!(
            fs::read_to_string(root.path().join("source.txt")).unwrap(),
            "original"
        );
        assert!(!root.path().join("dest/target.txt").exists());
    }
}

#[cfg(windows)]
pub(super) fn update_index(project: &Project, changed: &[&str]) -> Value {
    use serde_json::json;
    let path = match project.target_optional(".codegraph/codegraph.db") {
        Ok(path) => path,
        Err((code, _)) => return json!({"status":"needs_sync","code":code}),
    };
    match path.try_exists() {
        Ok(false) => return json!({"status":"not_present"}),
        Err(_) => return json!({"status":"needs_sync","code":"IO"}),
        Ok(true) => {}
    }
    // Reuse the indexer's deletion/addition and incoming-reference repair logic.
    let paths = changed.iter().map(|path| (*path).to_owned()).collect();
    let context = OperationContext::default();
    let scope_change = changed
        .iter()
        .any(|path| path.ends_with(".json") || path.ends_with(".gitignore"));
    let synced = if scope_change {
        super::index_store::sync_workspace_all(&project.root, &context)
    } else {
        super::index_store::sync_changed_paths(&project.root, &paths, &context)
    };
    match synced {
        Ok(report) => {
            json!({"status":if report["failure"].is_null() && report["selectedScopeApplied"] == true {
            "updated"
        } else { "needs_sync" },"report":report})
        }
        Err((code, _)) => json!({"status":"needs_sync","code":code}),
    }
}

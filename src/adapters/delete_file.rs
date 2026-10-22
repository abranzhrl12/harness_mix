//! Single-file deletion committed against an opened identity, with pinned ancestors.
use super::project::Project;
use harness_domain::{Failure, contracts::DeleteArgs, execution::OperationContext};
use serde_json::Value;

pub(super) fn delete_file(
    project: &Project,
    args: &DeleteArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    #[cfg(windows)]
    {
        delete_windows(project, args, context, || {})
    }
    #[cfg(not(windows))]
    {
        let _ = (project, args);
        Err((
            "UNSUPPORTED",
            "Deletion requires Windows path pinning in this release",
        ))
    }
}

#[cfg(windows)]
fn delete_windows(
    project: &Project,
    args: &DeleteArgs,
    context: &OperationContext,
    before_commit: impl FnOnce(),
) -> Result<Value, Failure> {
    use serde_json::json;
    use std::{
        fs::{self, File},
        io::Read,
        os::windows::{
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawHandle,
        },
    };
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_DISPOSITION_INFO, FileDispositionInfo,
            GetFileInformationByHandle, SetFileInformationByHandle,
        },
    };

    if args.path.len() > 1000 {
        return Err(("PATH", "Delete path exceeds 1000 bytes"));
    }
    if args
        .expected_hash
        .as_ref()
        .is_some_and(|h| h.len() != 64 || !h.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(("ARGUMENT", "expectedHash must be a 64-character SHA-256"));
    }
    let target = project
        .target(&args.path)?
        .canonicalize()
        .map_err(|_| ("PATH", "Delete target unavailable"))?;
    let relative = target
        .strip_prefix(&project.root)
        .map_err(|_| ("PATH", "Outside workspace"))?
        .to_string_lossy()
        .replace('\\', "/");
    if relative.is_empty() || super::move_file::protected(&relative) {
        return Err((
            "PATH",
            "Protected harness, instruction, index or version control path",
        ));
    }
    let _pins = super::edit_commit::pin_directory(
        project,
        target.parent().ok_or(("PATH", "Missing parent"))?,
    )?;
    project.target(&args.path)?;
    let read_access = if args.expected_hash.is_some() {
        0x8000_0000
    } else {
        0
    };
    let mut file = File::options()
        .access_mode(0x0001_0000 | 0x0080 | read_access)
        .share_mode(1) // Readers allowed; competing writes/deletes/renames excluded.
        .custom_flags(0x0020_0000) // OPEN_REPARSE_POINT: never follow a swapped link.
        .open(&target)
        .map_err(|_| {
            (
                "EDIT_BUSY",
                "Cannot acquire exclusive delete access to file",
            )
        })?;
    let metadata = file
        .metadata()
        .map_err(|_| ("IO", "Cannot inspect delete target"))?;
    if !metadata.is_file()
        || metadata.permissions().readonly()
        || metadata.file_attributes() & 0x400 != 0
    {
        return Err((
            "PATH",
            "Target must be a writable regular file without reparse points",
        ));
    }
    let mut identity = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the live file owns the handle and identity is writable.
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut identity) }
        .map_err(|_| ("IO", "Cannot inspect file identity"))?;
    if identity.nNumberOfLinks != 1 {
        return Err(("PATH", "Hard-linked files cannot be deleted"));
    }
    if let Some(expected) = &args.expected_hash {
        // Bound memory when a hash was requested; deletion without a hash does not read contents.
        const MAX_HASH_BYTES: u64 = 64 * 1024 * 1024;
        if metadata.len() > MAX_HASH_BYTES {
            return Err(("ARGUMENT", "expectedHash supports files up to 64 MiB"));
        }
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 64 * 1024];
        loop {
            context.check()?;
            let count = file
                .read(&mut chunk)
                .map_err(|_| ("IO", "Cannot hash delete target"))?;
            if count == 0 {
                break;
            }
            if bytes.len() as u64 + count as u64 > MAX_HASH_BYTES {
                return Err(("ARGUMENT", "expectedHash supports files up to 64 MiB"));
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        if !harness_domain::hash(&bytes).eq_ignore_ascii_case(expected) {
            return Err(("STALE", "File hash changed; reread before deleting"));
        }
    }
    before_commit();
    context.check()?;
    let disposition = FILE_DISPOSITION_INFO {
        DeleteFile: true.into(),
    };
    // SAFETY: the handle has DELETE access; disposition is valid for the supplied size.
    // Commit the opened identity rather than resolving the pathname again.
    unsafe {
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    }
    .map_err(|_| ("DELETE_IO", "Deletion failed; file retained"))?;
    drop(file);
    // After commit, cancellation/index errors must never hide the deletion.
    let verified = fs::symlink_metadata(&target)
        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound);
    let index = super::move_file::update_index(project, &[&relative]);
    let mut result = json!({"path":relative,"changed":true,"commitStatus":"committed",
        "verified":verified,"permanent":true,"index":index});
    let mut diagnostics = Vec::new();
    if !verified {
        result["ok"] = json!(false);
        diagnostics.push(json!({"code":"DELETE_POSTCHECK","message":"Delete committed but absence is unverified; inspect the path before any retry"}));
    }
    if result["index"]["status"] == "needs_sync" {
        diagnostics.push(json!({"code":"DELETE_INDEX","message":"Delete committed; index synchronization incomplete"}));
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
    fn pinned_identity_and_parents_prevent_swaps_and_cancel_before_commit() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("folder")).unwrap();
        let target = root.path().join("folder/file.txt");
        fs::write(&target, "original").unwrap();
        let project = Project::new(root.path()).unwrap();
        let args = DeleteArgs {
            path: "folder/file.txt".into(),
            expected_hash: None,
            session: None,
        };
        let context = OperationContext::default();
        let error = delete_windows(&project, &args, &context, || {
            assert!(fs::write(&target, "changed").is_err());
            assert!(fs::rename(&target, root.path().join("swapped.txt")).is_err());
            assert!(fs::rename(root.path().join("folder"), root.path().join("swapped")).is_err());
            context.cancel();
        })
        .unwrap_err();
        assert_eq!(error.0, "CANCELLED");
        assert_eq!(fs::read_to_string(target).unwrap(), "original");
    }
}

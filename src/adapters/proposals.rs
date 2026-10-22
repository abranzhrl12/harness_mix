//! Opt-in preparation only. Existing publication paths never call this module.
use super::project::Project;
use harness_domain::{Failure, contracts::ProposalArgs, execution::OperationContext};
use serde_json::Value;

pub(super) fn prepare(
    project: &Project,
    args: &ProposalArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    #[cfg(windows)]
    {
        platform::prepare(project, args, context, || {})
    }
    #[cfg(not(windows))]
    {
        let _ = (project, args);
        Err((
            "UNSUPPORTED",
            "Proposal persistence requires Windows path pinning",
        ))
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use crate::adapters::{edit, edit_commit, edit_security, move_file};
    use harness_domain::{editing, hash};
    use serde_json::json;
    use std::os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    };
    use std::{
        fs::{self, File},
        io::{Read, Write},
        path::{Path, PathBuf},
    };
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle},
    };

    const STORE: Failure = (
        "PROPOSAL_STORE",
        "Cannot save proposal; source file is unchanged",
    );
    const PATH: Failure = (
        "PROPOSAL_PATH",
        "Unsafe proposal storage; no existing artifact replaced",
    );

    fn inspect(file: &File) -> Result<(), Failure> {
        let metadata = file.metadata().map_err(|_| PATH)?;
        if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
            return Err(PATH);
        }
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: file owns a live handle and info is writable for this call.
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(|_| PATH)?;
        if info.nNumberOfLinks != 1 {
            return Err(PATH);
        }
        Ok(())
    }

    // Runtime directories are created only when explicitly preparing a valid proposal.
    fn directory(
        project: &Project,
        context: &OperationContext,
    ) -> Result<(PathBuf, Vec<File>), Failure> {
        let mut directory = project.root.clone();
        let mut pins = edit_commit::pin_directory(project, &directory).map_err(|_| PATH)?;
        for component in [".agents", "harness", "proposals"] {
            context.check()?;
            directory.push(component);
            match fs::create_dir(&directory) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(STORE),
            }
            // Never traverse junctions or links, including ones created concurrently.
            pins.push(edit_commit::pin_one_directory(&directory).map_err(|_| PATH)?);
        }
        Ok((directory, pins))
    }

    fn ignore(directory: &Path) -> Result<File, Failure> {
        let path = directory.join(".gitignore");
        let mut file = match File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .custom_flags(0x0020_0000)
            .share_mode(1)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(b"*\n").map_err(|_| STORE)?;
                file.sync_all().map_err(|_| STORE)?;
                return Ok(file);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => File::options()
                .read(true)
                .custom_flags(0x0020_0000)
                .share_mode(1)
                .open(&path)
                .map_err(|_| PATH)?,
            Err(_) => return Err(STORE),
        };
        inspect(&file)?;
        let mut bytes = Vec::new();
        (&mut file)
            .take(8)
            .read_to_end(&mut bytes)
            .map_err(|_| PATH)?;
        if !matches!(bytes.as_slice(), b"*\n" | b"*\r\n") {
            return Err(PATH);
        }
        Ok(file)
    }

    pub(super) fn prepare(
        project: &Project,
        args: &ProposalArgs,
        context: &OperationContext,
        before_store: impl FnOnce(),
    ) -> Result<Value, Failure> {
        context.check()?;
        if args.path.len() > 1000 || args.content.len() > editing::MAX_FILE {
            return Err((
                "SIZE",
                "Proposal path exceeds 1000 bytes or candidate exceeds 2 MiB",
            ));
        }
        if args.content.contains('\0') {
            return Err(("ENCODING", "Candidate must be UTF-8 text without NUL"));
        }
        if args
            .expected_hash
            .as_ref()
            .is_some_and(|h| h.len() != 64 || !h.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(("ARGUMENT", "expectedHash must be a 64-character SHA-256"));
        }
        let target = edit_commit::target(project, &args.path)?
            .canonicalize()
            .map_err(|_| ("PATH", "Proposal source unavailable"))?;
        let relative = target
            .strip_prefix(&project.root)
            .map_err(|_| ("PATH", "Outside workspace"))?
            .to_string_lossy()
            .replace('\\', "/");
        if relative.is_empty() || move_file::protected(&relative) {
            return Err((
                "PATH",
                "Protected harness, instruction, index or version control path",
            ));
        }
        let _source_pins = edit_commit::pin_parents(project, &target)?;
        project.target(&args.path)?;
        // Keep this exact source stable through snapshotting and durable preparation.
        let mut source = File::options()
            .read(true)
            .share_mode(1)
            .custom_flags(0x0020_0000)
            .open(&target)
            .map_err(|_| ("EDIT_BUSY", "Cannot acquire stable proposal source"))?;
        inspect(&source)?;
        if source
            .metadata()
            .map_err(|_| ("IO", "Cannot inspect source"))?
            .len()
            > editing::MAX_FILE as u64
        {
            return Err(("SIZE", "Proposal source exceeds 2 MiB"));
        }
        let mut bytes = Vec::new();
        (&mut source)
            .take(editing::MAX_FILE as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ("IO", "Cannot snapshot source"))?;
        if bytes.len() > editing::MAX_FILE {
            return Err(("SIZE", "Proposal source exceeds 2 MiB"));
        }
        let original = std::str::from_utf8(&bytes)
            .map_err(|_| ("ENCODING", "Proposal source must be UTF-8"))?;
        if original.contains('\0') {
            return Err(("ENCODING", "Binary proposal source is unsupported"));
        }
        let base_hash = hash(&bytes);
        if args
            .expected_hash
            .as_ref()
            .is_some_and(|expected| !base_hash.eq_ignore_ascii_case(expected))
        {
            return Err((
                "STALE",
                "Source changed since expectedHash; no proposal saved",
            ));
        }
        let candidate = editing::utf8_bom(&relative, &args.content, args.bom, Some(&bytes))?;
        let syntax = edit::syntax_allowing_unvalidated(
            &relative,
            &candidate,
            args.allow_unvalidated_text,
            context,
        )?;
        let candidate_hash = hash(candidate.as_bytes());
        if bytes == candidate.as_bytes() {
            return Ok(
                json!({"path":relative,"status":"unchanged","stored":false,"proposalId":null,
                "changed":false,"published":false,"candidateChanged":false,
                "baseHash":base_hash,"candidateHash":candidate_hash,"syntax":syntax,"patch":""}),
            );
        }
        context.check()?;
        let patch = edit::preview_diff(&relative, original, &candidate);
        let id = uuid::Uuid::new_v4().to_string();
        let created_at = chrono::Utc::now().to_rfc3339();
        let record = json!({"format":"agent-harness-proposal-v1","proposalId":id,"status":"pending",
            "operation":"replace","path":relative,"createdAt":created_at,
            "baseHash":base_hash,"candidateHash":candidate_hash,
            "original":original,"candidate":candidate,"syntax":syntax});
        let serialized = serde_json::to_vec(&record).map_err(|_| STORE)?;
        context.check()?;
        let (directory, _storage_pins) = directory(project, context)?;
        let _ignore = ignore(&directory)?;
        let mut stage = tempfile::Builder::new()
            .prefix(".prepare-")
            .tempfile_in(&directory)
            .map_err(|_| STORE)?;
        // Apply source permissions before writing its contents to runtime storage.
        edit_security::protect_stage(&target, stage.path())?;
        stage.write_all(&serialized).map_err(|_| STORE)?;
        stage.as_file().sync_all().map_err(|_| STORE)?;
        before_store();
        context.check()?;
        let artifact = directory.join(format!("{id}.json"));
        stage.persist_noclobber(&artifact).map_err(|_| STORE)?;
        // Persist is the commit point; cancellation afterward must not hide this ID.
        let truncated = patch.chars().count() > 12000;
        Ok(
            json!({"path":relative,"status":"pending","stored":true,"proposalId":id,
            "changed":false,"published":false,"candidateChanged":true,"createdAt":created_at,
            "baseHash":base_hash,"candidateHash":candidate_hash,"syntax":syntax,
            "storagePath":format!(".agents/harness/proposals/{id}.json"),
            "patch":patch.chars().take(12000).collect::<String>(),"truncated":truncated,
            "notice":"Proposal saved only; the source is unchanged. Accept/reject are not implemented. Repeated preparations create separate IDs.",
            "_complete":!truncated,"_detail":{"patch":patch,"syntax":syntax}}),
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn proposal_storage_does_not_change_source_fingerprint_or_enter_index() {
            let root = tempfile::tempdir().unwrap();
            fs::write(root.path().join("a.txt"), "old").unwrap();
            let project = Project::new(root.path()).unwrap();
            let context = OperationContext::default();
            let before = crate::adapters::fingerprint::capture(&project, &context).unwrap();
            let args =
                serde_json::from_value(json!({"action":"prepare","path":"a.txt","content":"new"}))
                    .unwrap();
            let result = prepare(&project, &args, &context, || {}).unwrap();
            assert_eq!(result["stored"], true);
            assert_eq!(
                before,
                crate::adapters::fingerprint::capture(&project, &context).unwrap()
            );
            assert!(crate::adapters::index_inventory::restricted(
                result["storagePath"].as_str().unwrap()
            ));
            assert!(crate::adapters::index_inventory::restricted(
                ".AGENTS\\HARNESS\\PROPOSALS\\a.json"
            ));
            assert!(!crate::adapters::index_inventory::restricted(
                "src/proposals/a.ts"
            ));
        }

        #[test]
        fn persisted_snapshot_preserves_original_access_control() {
            let root = tempfile::tempdir().unwrap();
            let target = root.path().join("a.txt");
            fs::write(&target, "old").unwrap();
            edit_security::protect_stage(&target, &target).unwrap();
            let original_descriptor = edit_security::descriptor(&target).unwrap();
            let project = Project::new(root.path()).unwrap();
            let args =
                serde_json::from_value(json!({"action":"prepare","path":"a.txt","content":"new"}))
                    .unwrap();
            let result = prepare(&project, &args, &OperationContext::default(), || {}).unwrap();
            let stored = root.path().join(result["storagePath"].as_str().unwrap());
            assert_eq!(
                edit_security::descriptor(&stored).unwrap(),
                original_descriptor
            );
            assert_eq!(
                edit_security::descriptor(&target).unwrap(),
                original_descriptor
            );
        }

        #[test]
        fn preparation_pins_source_and_cancellation_discards_staging_file() {
            let root = tempfile::tempdir().unwrap();
            let target = root.path().join("a.txt");
            fs::write(&target, "old").unwrap();
            let project = Project::new(root.path()).unwrap();
            let args =
                serde_json::from_value(json!({"action":"prepare","path":"a.txt","content":"new"}))
                    .unwrap();
            let context = OperationContext::default();
            let error = prepare(&project, &args, &context, || {
                assert!(fs::write(&target, "external").is_err());
                assert!(fs::rename(&target, root.path().join("moved.txt")).is_err());
                context.cancel();
            })
            .unwrap_err();
            assert_eq!(error.0, "CANCELLED");
            assert_eq!(fs::read_to_string(&target).unwrap(), "old");
            let files = fs::read_dir(root.path().join(".agents/harness/proposals"))
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<Vec<_>>();
            assert_eq!(files, vec![std::ffi::OsString::from(".gitignore")]);
        }
    }
}

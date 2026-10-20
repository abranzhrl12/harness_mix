use super::{edit, edit_commit, project::Project};
use harness_domain::{
    Failure,
    contracts::{ReplaceArgs, WriteArgs},
    editing,
    execution::OperationContext,
    hash,
};
use serde_json::{Value, json};

pub(super) fn write(
    project: &Project,
    args: &WriteArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    let mut result = write_with(project, args, context, |_| {})?;
    if result["ok"] == true {
        super::csproj::attach_warning(project, &args.path, &mut result, context);
    }
    Ok(result)
}

fn write_with(
    project: &Project,
    args: &WriteArgs,
    context: &OperationContext,
    before_publish: impl FnOnce(&std::path::Path),
) -> Result<Value, Failure> {
    context.check()?;
    #[cfg(not(windows))]
    {
        let _ = (project, args, before_publish);
        return Err(("UNSUPPORTED", "Creation requires Windows ancestor pinning"));
    }
    #[cfg(windows)]
    {
        use std::{fs, io::Write};
        if args.path.len() > 4096 || args.content.len() > editing::MAX_FILE {
            return Err((
                "SIZE",
                "Path exceeds 4096 bytes or content exceeds 2 MiB; MCP message budget is 128 KiB",
            ));
        }
        if args.content.contains('\0') {
            return Err(("ENCODING", "Binary files cannot be created"));
        }
        let content = editing::utf8_bom(&args.path, &args.content, args.bom, None)?;
        let target = project.target_optional(&args.path)?;
        let relative = target
            .strip_prefix(&project.root)
            .map_err(|_| ("PATH", "Outside workspace"))?;
        if relative.components().count() > 33 {
            return Err(("PATH", "Creation supports at most 32 parent levels"));
        }
        let knowledge_document = edit_commit::is_agent_knowledge_document(&args.path);
        if relative.components().enumerate().any(|(index, part)| {
            let part = part.as_os_str().to_string_lossy();
            [".git", ".hg", ".svn", ".codegraph", "agents.md"]
                .iter()
                .any(|name| part.eq_ignore_ascii_case(name))
                || (part.eq_ignore_ascii_case(".agents") && !(knowledge_document && index == 0))
        }) {
            return Err((
                "PATH",
                "Protected instruction, index or version control path",
            ));
        }
        let validation = match edit::syntax(&args.path, &content, context) {
            Ok(value) => value,
            Err(("PARSER_UNAVAILABLE", _)) if args.allow_unvalidated_text => {
                json!({"status":"not_checked","validation":"none","reason":"parser_unavailable","explicitlyAllowed":true})
            }
            Err((code, message)) => return Ok(validation_error(args, code, message)),
        };
        let parent = target.parent().ok_or(("PATH", "Missing parent"))?;
        let mut created = Vec::new();
        let mut pins = edit_commit::pin_directory(project, &project.root)?;
        let attempt = (|| {
            let mut current = project.root.clone();
            for part in parent
                .strip_prefix(&project.root)
                .map_err(|_| ("PATH", "Outside workspace"))?
                .components()
            {
                context.check()?;
                current.push(part);
                project.target_optional(&args.path)?;
                match fs::symlink_metadata(&current) {
                    Ok(_) => {}
                    Err(error)
                        if error.kind() == std::io::ErrorKind::NotFound && args.create_parents =>
                    {
                        match fs::create_dir(&current) {
                            Ok(()) => created.push(
                                current
                                    .strip_prefix(&project.root)
                                    .unwrap()
                                    .to_string_lossy()
                                    .replace('\\', "/"),
                            ),
                            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                            Err(_) => return Err(("IO", "Cannot create parent directory")),
                        }
                    }
                    Err(_) => {
                        return Err((
                            "PATH",
                            "Parent is inaccessible or missing; enable createParents for missing directories",
                        ));
                    }
                }
                // The previous parent remains pinned while this child is created/opened.
                pins.push(edit_commit::pin_one_directory(&current)?);
            }
            project.target_optional(&args.path)?;
            context.check()?;
            if target
                .try_exists()
                .map_err(|_| ("PATH", "Cannot inspect destination"))?
            {
                let bytes = project.bytes_with_context(&args.path, context)?;
                let content = editing::utf8_bom(&args.path, &args.content, args.bom, Some(&bytes))?;
                let expected_hash_opt = if let Some(expected) = &args.expected_hash {
                    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
                        return Err((
                            "ARGUMENT",
                            "expectedHash must be a 64-character SHA-256 hex digest",
                        ));
                    }
                    if &hash(&bytes) != expected {
                        return Err((
                            "STALE",
                            "Destination changed since expectedHash was computed",
                        ));
                    }
                    Some(expected.clone())
                } else if args.overwrite {
                    Some(hash(&bytes))
                } else {
                    None
                };

                if let Some(expected) = expected_hash_opt {
                    if bytes == content.as_bytes() {
                        return Ok(
                            json!({"path":args.path,"status":"unchanged","created":false,
                            "changed":false,"commitStatus":"not_needed","previousHash":expected,
                            "hash":expected,"verified":true,"ok":true,"syntax":validation,
                            "diagnostics":[],"typecheck":"not_run",
                            "changeSummary":{"addedLines":0,"removedLines":0}}),
                        );
                    }
                    let replace_args = ReplaceArgs {
                        path: args.path.clone(),
                        bom: args.bom,
                        hash: expected.clone(),
                        line: None,
                        chunks: None,
                        old_text: String::new(),
                        new_text: content.clone(),
                        allow_unvalidated_text: args.allow_unvalidated_text,
                        preview: false,
                        session: args.session.clone(),
                    };
                    let mut result =
                        edit_commit::publish(project, &replace_args, &target, &content, context)?;
                    result["path"] = json!(args.path);
                    result["created"] = json!(false);
                    result["previousHash"] = json!(expected);
                    result["syntax"] = validation.clone();
                    result["typecheck"] = json!("not_run");
                    result["changeSummary"] = json!({
                        "bytesBefore":bytes.len(),"bytesAfter":content.len(),
                        "removedLines":std::str::from_utf8(&bytes).map(|t| t.lines().count()).unwrap_or(0),
                        "addedLines":content.lines().count()});
                    if result["changed"] == true {
                        let expected_after = hash(content.as_bytes());
                        let verified = project
                            .bytes_with_context(&args.path, &OperationContext::default())
                            .is_ok_and(|bytes| hash(&bytes) == expected_after);
                        result["status"] = json!("replaced");
                        result["hash"] = json!(expected_after);
                        result["verified"] = json!(verified);
                        result["ok"] = json!(verified);
                        if !verified {
                            result["diagnostics"] = json!([{"code":"EDIT_POSTCHECK","path":args.path,
                                "message":"File was published but postcheck failed; inspect before retrying"}]);
                        }
                    } else {
                        result["status"] = json!("unchanged");
                        result["ok"] = json!(false);
                    }
                    if result["diagnostics"].is_null() {
                        result["diagnostics"] = json!([]);
                    }
                    return Ok(result);
                }
                if bytes != content.as_bytes() {
                    return Err((
                        "WRITE_EXISTS",
                        "Destination has different bytes; set overwrite: true or use harness_replace",
                    ));
                }
                return Ok(
                    json!({"path":args.path,"status":"already_present","created":false,"changed":false,
                    "commitStatus":"not_needed","hash":hash(&bytes),"verified":true,"ok":true,"syntax":validation,
                    "diagnostics":[],"typecheck":"not_run","changeSummary":{"addedLines":0,"removedLines":0}}),
                );
            }
            if args.expected_hash.is_some() {
                return Err((
                    "PATH",
                    "expectedHash was provided but the destination does not exist; omit it to create the file",
                ));
            }
            let mut stage = tempfile::Builder::new()
                .prefix(".harness-create-")
                .tempfile_in(parent)
                .map_err(|_| ("IO", "Cannot stage new file"))?;
            stage
                .write_all(content.as_bytes())
                .map_err(|_| ("IO", "Cannot write new file"))?;
            stage
                .as_file()
                .sync_all()
                .map_err(|_| ("IO", "Cannot sync new file"))?;
            before_publish(&target);
            context.check()?;
            stage.persist_noclobber(&target).map_err(|error| {
                if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                    (
                        "WRITE_EXISTS",
                        "Destination was created concurrently; retry the same request safely",
                    )
                } else {
                    ("IO", "Cannot publish new file")
                }
            })?;
            let expected = hash(content.as_bytes());
            let verified = project
                .bytes_with_context(&args.path, &OperationContext::default())
                .is_ok_and(|bytes| hash(&bytes) == expected);
            let diagnostics = if verified {
                json!([])
            } else {
                json!([{"code":"EDIT_POSTCHECK","path":args.path,"message":"File was published but postcheck failed; inspect before retrying"}])
            };
            Ok(
                json!({"path":args.path,"status":"created","created":true,"changed":true,"commitStatus":"committed",
                "hash":expected,"verified":verified,"ok":verified,"syntax":validation,"diagnostics":diagnostics,
                "typecheck":"not_run","changeSummary":{"addedLines":content.lines().count(),"removedLines":0,"bytesWritten":content.len()}}),
            )
        })();
        let mut result = match attempt {
            Ok(value) => value,
            Err((code, message)) => {
                json!({"path":args.path,"status":"not_created","ok":false,"created":false,
                "changed":false,"commitStatus":"not_committed","verified":false,"typecheck":"not_run",
                "cancelled":code == "CANCELLED","timedOut":code == "TIMEOUT",
                "diagnostics":[{"code":code,"path":args.path,"message":message}]})
            }
        };
        result["createdDirectories"] = json!(created);
        if result["syntax"].is_null() {
            result["syntax"] = validation;
        }
        result["filesystemChanged"] = json!(result["changed"] == true || !created.is_empty());
        // Never recursively remove directories on failure: another process may have used them.
        if result["ok"] == false && !created.is_empty() {
            result["cleanup"] = json!({"status":"directories_retained","reason":"No unsafe rollback of directories potentially used by other processes"});
        }
        Ok(result)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn cancellation_and_races_preserve_files_and_report_directory_side_effects() {
        for cancelled in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let project = Project::new(root.path()).unwrap();
            let context = OperationContext::default();
            let args: WriteArgs = serde_json::from_value(
                json!({"path":"new/deep/file.txt","content":"requested","createParents":true}),
            )
            .unwrap();
            let result = write_with(&project, &args, &context, |target| {
                assert!(
                    std::fs::rename(target.parent().unwrap(), root.path().join("moved")).is_err()
                );
                if cancelled {
                    context.cancel();
                } else {
                    std::fs::write(target, "external").unwrap();
                }
            })
            .unwrap();
            assert_eq!(result["ok"], false);
            assert_eq!(result["changed"], false);
            assert_eq!(result["commitStatus"], "not_committed");
            assert_eq!(
                result["diagnostics"][0]["code"],
                if cancelled {
                    "CANCELLED"
                } else {
                    "WRITE_EXISTS"
                }
            );
            assert_eq!(result["createdDirectories"], json!(["new", "new/deep"]));
            assert_eq!(result["cleanup"]["status"], "directories_retained");
            let path = root.path().join("new/deep/file.txt");
            if cancelled {
                assert!(!path.exists());
            } else {
                assert_eq!(std::fs::read_to_string(path).unwrap(), "external");
            }
            assert_eq!(
                std::fs::read_dir(root.path().join("new/deep"))
                    .unwrap()
                    .count(),
                usize::from(!cancelled)
            );
        }
    }

    #[test]
    fn test_write_overwrite_existing_file() {
        let root = tempfile::tempdir().unwrap();
        let project = Project::new(root.path()).unwrap();
        let context = OperationContext::default();
        let file_path = root.path().join("created.txt");
        std::fs::write(&file_path, "initial content").unwrap();

        // 1. Without overwrite, different bytes must fail with WRITE_EXISTS
        let args_fail: WriteArgs = serde_json::from_value(
            json!({"path":"created.txt","content":"updated content","overwrite":false,"allowUnvalidatedText":true}),
        )
        .unwrap();
        let res_fail = write(&project, &args_fail, &context).unwrap();
        assert_eq!(res_fail["ok"], false);
        assert_eq!(res_fail["status"], "not_created");
        assert_eq!(res_fail["diagnostics"][0]["code"], "WRITE_EXISTS");

        // 2. With overwrite: true, it must atomically overwrite and report replaced
        let args_ok: WriteArgs = serde_json::from_value(
            json!({"path":"created.txt","content":"updated content","overwrite":true,"allowUnvalidatedText":true}),
        )
        .unwrap();
        let res_ok = write(&project, &args_ok, &context).unwrap();
        assert_eq!(res_ok["ok"], true);
        assert_eq!(res_ok["status"], "replaced");
        assert_eq!(
            std::fs::read_to_string(&file_path).unwrap(),
            "updated content"
        );
    }
}

fn validation_error(args: &WriteArgs, code: &str, message: &str) -> Value {
    let mut diagnostic = json!({"code":code,"path":args.path,"message":message});
    if code == "SYNTAX_ERROR"
        && std::path::Path::new(&args.path)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    {
        if let Err(error) = serde_json::from_str::<Value>(
            args.content
                .strip_prefix('\u{feff}')
                .unwrap_or(&args.content),
        ) {
            diagnostic["line"] = json!(error.line());
            diagnostic["column"] = json!(error.column());
            diagnostic["positionUnits"] = json!("one_based_line_and_byte_column");
        }
    }
    json!({"path":args.path,"status":"not_created","ok":false,"changed":false,"created":false,
        "commitStatus":"not_committed","createdDirectories":[],"filesystemChanged":false,
        "syntax":{"status":"error","validation":"syntax_only"},"typecheck":"not_run",
        "cancelled":code == "CANCELLED","timedOut":code == "TIMEOUT","diagnostics":[diagnostic]})
}

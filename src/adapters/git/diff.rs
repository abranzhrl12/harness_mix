use super::reader::Reader;
use crate::adapters::project::Project;
use harness_domain::{
    Failure,
    contracts::GitDiffArgs,
    execution::OperationContext,
    results::{prefix, utf16_len},
};
use serde_json::{Value, json};
use std::time::Instant;

pub(super) fn diff(
    project: &Project,
    args: &GitDiffArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    let comparison = args
        .comparison
        .as_deref()
        .unwrap_or(if args.staged == Some(true) {
            "HEAD:index"
        } else {
            "index:worktree"
        });
    if comparison == "auto" {
        let mut comparisons = serde_json::Map::new();
        let mut changed = false;
        for mode in ["HEAD:index", "index:worktree", "HEAD:worktree"] {
            let selected = GitDiffArgs {
                path: args.path.clone(),
                staged: args.staged,
                comparison: Some(mode.into()),
                max_lines: args.max_lines,
                max_chars: args.max_chars,
                session: args.session.clone(),
            };
            let result = diff(project, &selected, context)?;
            changed |= result["changed"] == true;
            comparisons.insert(mode.into(), result);
        }
        return Ok(
            json!({"path":args.path,"comparison":"auto","changed":changed,"comparisons":comparisons}),
        );
    }
    if !["HEAD:index", "index:worktree", "HEAD:worktree"].contains(&comparison) {
        return Err((
            "ARGUMENT",
            "Git comparison must be auto, HEAD:index, index:worktree or HEAD:worktree",
        ));
    }
    let lines_limit = args.max_lines.unwrap_or(120);
    let chars = args.max_chars.unwrap_or(6000);
    if !(1..=500).contains(&lines_limit)
        || !(500..=12000).contains(&chars)
        || args.path.len() > 1000
    {
        return Err((
            "ARGUMENT",
            "Git diff maxLines 1..500, maxChars 500..12000, path <=1000 bytes",
        ));
    }
    let normalized = args.path.replace('\\', "/");
    for component in normalized.split('/') {
        let lower = component.to_ascii_lowercase();
        if lower == ".git"
            || lower == ".npmrc"
            || lower == "nuget.config"
            || (lower.starts_with("web") && lower.ends_with(".config"))
            || (lower.starts_with("appsettings") && lower.ends_with(".json"))
        {
            return Err((
                "PATH",
                "Git metadata and connection/credential configuration cannot be diffed",
            ));
        }
    }
    if normalized.is_empty() {
        return Err((
            "ARGUMENT",
            "Git diff requires path; use path: \".\" for the workspace or path: \"src\" for a directory",
        ));
    }
    let file = if normalized == "." {
        project.root.clone()
    } else {
        project.target_optional(&normalized)?
    };
    let started = Instant::now();
    let reader = Reader::new(project, context)?;
    // Enumerate first: a directory pathspec must never disclose protected files.
    // Literal pathspecs also prevent user paths from becoming Git magic/globs.
    // Reader already sets --literal-pathspecs globally.
    let literal_scope = normalized.clone();
    let mut names_args = vec![
        "diff",
        "--name-only",
        "-z",
        "--no-ext-diff",
        "--no-textconv",
        "--no-renames",
        "--ignore-submodules=all",
    ];
    if comparison == "HEAD:index" {
        names_args.push("--cached");
    } else if comparison == "HEAD:worktree" {
        names_args.push("HEAD");
    }
    names_args.extend(["--", &literal_scope]);
    let names = reader.execute(&names_args, false)?;
    let mut selected = Vec::new();
    let mut excluded = 0;
    let mut files_omitted = 0;
    let mut argument_bytes = 0;
    let mut total_files = 0;
    for path in names.split('\0').filter(|p| !p.is_empty()) {
        context.check()?;
        total_files += 1;
        let allowed = !path.split('/').any(|part| {
            let lower = part.to_ascii_lowercase();
            lower == ".git"
                || lower == ".npmrc"
                || lower == "nuget.config"
                || (lower.starts_with("web") && lower.ends_with(".config"))
                || (lower.starts_with("appsettings") && lower.ends_with(".json"))
        }) && project
            .target_optional(path)
            .is_ok_and(|target| !target.exists() || target.is_file());
        if !allowed {
            excluded += 1;
            continue;
        }
        if selected.len() >= 128 || argument_bytes + path.len() + 16 > 24000 {
            files_omitted += 1;
            continue;
        }
        argument_bytes += path.len() + 16;
        selected.push(path.to_string());
    }
    let mut args_list = vec!["diff"];
    if comparison == "HEAD:index" {
        args_list.push("--cached");
    } else if comparison == "HEAD:worktree" {
        args_list.push("HEAD");
    }
    args_list.extend([
        "--no-ext-diff",
        "--no-textconv",
        "--no-renames",
        "--ignore-submodules=all",
        "--no-color",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        "--unified=3",
        "--",
    ]);
    args_list.extend(selected.iter().map(String::as_str));
    let patch = if selected.is_empty() {
        String::new()
    } else {
        reader.execute(&args_list, false)?
    };
    let lines: Vec<_> = patch
        .trim_end_matches('\n')
        .split('\n')
        .filter(|_| !patch.is_empty())
        .collect();
    let joined = lines
        .iter()
        .take(lines_limit)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    let preview = prefix(&joined, chars);
    let truncated = lines.len() > lines_limit || utf16_len(&joined) > chars || files_omitted > 0;
    let hunks: Vec<_> = lines
        .iter()
        .filter(|s| s.starts_with("@@"))
        .take(20)
        .map(|s| prefix(s, 300))
        .collect();
    let mut result = json!({"path":normalized,"comparison":comparison,"changed":!patch.is_empty(),"binary":lines.iter().any(|s|s.starts_with("Binary files ")),"hunks":hunks,"totalLines":lines.len(),"preview":preview,"milliseconds":started.elapsed().as_millis() as u64,
        "note":"Tracked changes only; textual diff, not semantic proof. Hunk labels come from Git."});
    if comparison == "index:worktree" && patch.is_empty() {
        let staged = reader.execute(
            &[
                "diff",
                "--cached",
                "--name-only",
                "-z",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--ignore-submodules=all",
                "--",
                &literal_scope,
            ],
            false,
        )?;
        if !staged.is_empty() {
            result["stagedChangesAvailable"] = json!(true);
            result["warning"] = json!(
                "No hay cambios sin stage, pero sí hay cambios staged. Use comparison: HEAD:index o auto."
            );
        }
    }
    result["scope"] = json!(if normalized == "." {
        "workspace"
    } else if file.is_dir() {
        "directory"
    } else {
        "path"
    });
    result["files"] = json!({"changed":total_files,"included":selected.len(),"excludedByPolicy":excluded,"omittedByLimit":files_omitted,"limit":128});
    result["complete"] = json!(files_omitted == 0 && excluded == 0);
    if truncated {
        result["truncated"] = json!(true);
        result["_detail"] = json!({"path":normalized,"comparison":comparison,"patch":patch});
    }
    Ok(result)
}

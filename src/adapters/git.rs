//! Read-only Git adapter. No caller-selected command, shell, external diff or filters.
use super::{process::NativeProcessRunner, project::Project};
use harness_application::ports::ProcessRunner;
use harness_domain::{
    Failure,
    contracts::{GitDiffArgs, GitStatusArgs},
    execution::OperationContext,
    process::{ProcessSpec, Program, StopReason},
    results::{prefix, utf16_len},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const METADATA_ERROR: Failure = ("GIT_REPO", "Invalid or unsupported Git worktree metadata");

// Git worktree metadata may live outside the workspace. Authorize only the
// reciprocal .git <-> common/worktrees/id registration, never general file access.
fn metadata_path(path: &Path) -> Result<PathBuf, Failure> {
    if path.components().count() > 128 {
        return Err(METADATA_ERROR);
    }
    // Inspect the original spelling before canonicalization, including ancestors
    // before `..`, so canonicalization cannot hide a link or reparse point.
    for ancestor in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        let meta = std::fs::symlink_metadata(ancestor).map_err(|_| METADATA_ERROR)?;
        if meta.file_type().is_symlink() {
            return Err(METADATA_ERROR);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(METADATA_ERROR);
            }
        }
    }
    path.canonicalize().map_err(|_| METADATA_ERROR)
}

fn metadata_reference(base: &Path, text: &str) -> Result<PathBuf, Failure> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    let text = text.strip_suffix('\r').unwrap_or(text);
    if text.is_empty() || text.len() > 4096 || text.chars().any(char::is_control) {
        return Err(METADATA_ERROR);
    }
    let path = Path::new(text);
    // Drive-relative paths are ambiguous on Windows; fully relative paths are OK.
    if !path.is_absolute()
        && path.components().any(|c| {
            matches!(
                c,
                std::path::Component::Prefix(_) | std::path::Component::RootDir
            )
        })
    {
        return Err(METADATA_ERROR);
    }
    metadata_path(&base.join(path))
}

fn metadata_text(
    directory: &Path,
    name: &str,
    context: &OperationContext,
) -> Result<String, Failure> {
    metadata_path(&directory.join(name))?;
    let project = Project::new(directory).map_err(|_| METADATA_ERROR)?;
    let bytes = project
        .bytes_with_limit(name, context, 8192)
        .map_err(|error| {
            if matches!(error.0, "CANCELLED" | "TIMEOUT") {
                error
            } else {
                METADATA_ERROR
            }
        })?;
    String::from_utf8(bytes).map_err(|_| METADATA_ERROR)
}

fn repository_directory(project: &Project, context: &OperationContext) -> Result<PathBuf, Failure> {
    let dotgit = project.target(".git").map_err(|_| METADATA_ERROR)?;
    if dotgit.is_dir() {
        return metadata_path(&dotgit);
    }
    let text = metadata_text(&project.root, ".git", context)?;
    let reference = text.strip_prefix("gitdir: ").ok_or(METADATA_ERROR)?;
    let directory = metadata_reference(&project.root, reference)?;
    if !directory.is_dir() {
        return Err(METADATA_ERROR);
    }
    let backlink = metadata_text(&directory, "gitdir", context)?;
    if metadata_reference(&directory, &backlink)? != metadata_path(&dotgit)? {
        return Err(METADATA_ERROR);
    }
    let common = metadata_text(&directory, "commondir", context)?;
    let common = metadata_reference(&directory, &common)?;
    if !common.is_dir()
        || directory.parent() != Some(metadata_path(&common.join("worktrees"))?.as_path())
    {
        return Err(METADATA_ERROR);
    }
    Ok(directory)
}

struct Reader<'a> {
    runner: NativeProcessRunner,
    context: &'a OperationContext,
    base: Vec<String>,
}
impl<'a> Reader<'a> {
    fn execute(&self, args: &[&str], absent: bool) -> Result<String, Failure> {
        self.context.check()?;
        let mut command = self.base.clone();
        command.extend(args.iter().map(|s| s.to_string()));
        let spec = ProcessSpec::new(
            Program::Git,
            command,
            Duration::from_secs(15),
            2 * 1024 * 1024,
        )?;
        let result = self.runner.run(&spec, self.context)?;
        match result.reason {
            StopReason::Cancelled => return Err(("CANCELLED", "Git query cancelled")),
            StopReason::TimedOut => return Err(("TIMEOUT", "Git query timed out")),
            StopReason::OutputLimit => return Err(("GIT_SIZE", "Git output exceeded 2 MiB")),
            StopReason::Exited => {}
        }
        // stderr/config values may contain credentials: never forward them.
        if result.exit_code != Some(0) && !(absent && result.exit_code == Some(1)) {
            return Err((
                "GIT_READ",
                "Git cannot read this repository; check permissions and configuration",
            ));
        }
        String::from_utf8(result.stdout).map_err(|_| ("GIT_ENCODING", "Git output must be UTF-8"))
    }
    fn new(project: &Project, context: &'a OperationContext) -> Result<Self, Failure> {
        context.check()?;
        let directory = repository_directory(project, context)?;
        let mut reader = Self {
            runner: NativeProcessRunner::new(&project.root)?,
            context,
            base: [
                "--no-pager",
                "--no-optional-locks",
                "--literal-pathspecs",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "core.untrackedCache=false",
                "-c",
                "core.quotePath=false",
                "-c",
                "color.ui=false",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        };
        let root = reader.execute(&["rev-parse", "--show-toplevel"], false)?;
        let actual = Path::new(root.trim_end_matches(['\r', '\n']))
            .canonicalize()
            .map_err(|_| ("GIT_REPO", "Git root unavailable"))?;
        if actual != project.root {
            return Err((
                "GIT_REPO",
                "Git worktree differs from the authorized workspace",
            ));
        }
        let actual_directory = reader.execute(&["rev-parse", "--absolute-git-dir"], false)?;
        if metadata_path(Path::new(actual_directory.trim_end_matches(['\r', '\n'])))? != directory {
            return Err(METADATA_ERROR);
        }
        let names = reader.execute(
            &[
                "config",
                "--includes",
                "--null",
                "--name-only",
                "--get-regexp",
                r"^filter\..*\.(clean|process|required)$",
            ],
            true,
        )?;
        let mut filters = BTreeSet::new();
        for key in names.split('\0').filter(|s| !s.is_empty()) {
            if key.len() > 240 || key.contains('=') || key.chars().any(char::is_control) {
                return Err(("GIT_FILTERS", "Unsupported filter name; query refused"));
            }
            let (name, suffix) = key
                .rsplit_once('.')
                .ok_or(("GIT_FILTERS", "Invalid filter key"))?;
            if !key.to_ascii_lowercase().starts_with("filter.")
                || !["clean", "process", "required"].contains(&suffix.to_ascii_lowercase().as_str())
            {
                return Err(("GIT_FILTERS", "Invalid filter key"));
            }
            filters.insert(name.to_owned());
            if filters.len() > 12 {
                return Err((
                    "GIT_FILTERS",
                    "Too many configured filters for a bounded safe query",
                ));
            }
        }
        for name in filters {
            for value in [
                format!("{name}.clean="),
                format!("{name}.process="),
                format!("{name}.required=false"),
            ] {
                reader.base.extend(["-c".into(), value]);
            }
        }
        Ok(reader)
    }
}

fn parse_status(text: &str) -> Result<Value, Failure> {
    if !text.is_empty() && !text.ends_with('\0') {
        return Err(("GIT_FORMAT", "Incomplete Git status"));
    }
    let mut result = json!({"branch":null,"head":null,"upstream":null,"ahead":null,"behind":null,"detached":false,"unborn":false,"entries":[]});
    let mut entries = Vec::new();
    let mut records = text.split('\0');
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        if let Some(head) = record.strip_prefix("# branch.oid ") {
            result["unborn"] = json!(head == "(initial)");
            if head != "(initial)" {
                result["head"] = json!(head);
            }
        } else if let Some(branch) = record.strip_prefix("# branch.head ") {
            result["detached"] = json!(branch == "(detached)");
            if branch != "(detached)" {
                result["branch"] = json!(branch);
            }
        } else if let Some(upstream) = record.strip_prefix("# branch.upstream ") {
            result["upstream"] = json!(upstream);
        } else if let Some(ab) = record.strip_prefix("# branch.ab ") {
            let (a, b) = ab
                .split_once(' ')
                .ok_or(("GIT_FORMAT", "Invalid branch counts"))?;
            let parse = |s: Option<&str>| {
                s.and_then(|s| s.parse::<u64>().ok())
                    .ok_or(("GIT_FORMAT", "Invalid branch counts"))
            };
            result["ahead"] = json!(parse(a.strip_prefix('+'))?);
            result["behind"] = json!(parse(b.strip_prefix('-'))?);
        } else if let Some(path) = record.strip_prefix("? ") {
            entries.push(json!({"path":path,"xy":"??","kind":"untracked"}));
        } else if record.starts_with("# ") {
            continue;
        } else {
            let fields = match record.as_bytes()[0] {
                b'1' => 8,
                b'2' => 9,
                b'u' => 10,
                _ => return Err(("GIT_FORMAT", "Unknown status record")),
            };
            let parts: Vec<_> = record.splitn(fields + 1, ' ').collect();
            if parts.len() != fields + 1 || parts[1].len() != 2 || parts[fields].is_empty() {
                return Err(("GIT_FORMAT", "Incomplete status record"));
            }
            let mut entry = json!({"path":parts[fields],"xy":parts[1],"kind":if record.starts_with('u') {"conflict"} else {"tracked"}});
            if record.starts_with('2') {
                entry["from"] = json!(
                    records
                        .next()
                        .filter(|s| !s.is_empty())
                        .ok_or(("GIT_FORMAT", "Missing rename source"))?
                );
            }
            entries.push(entry);
        }
    }
    result["entries"] = json!(entries);
    Ok(result)
}

pub(super) fn status(
    project: &Project,
    args: &GitStatusArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    let limit = args.limit.unwrap_or(30);
    let offset = args.offset.unwrap_or(0);
    if !(1..=100).contains(&limit) || offset > 1000000 {
        return Err(("ARGUMENT", "Git limit 1..100, offset 0..1000000"));
    }
    let started = Instant::now();
    let reader = Reader::new(project, context)?;
    let mode = if args.include_untracked == Some(false) {
        "--untracked-files=no"
    } else {
        "--untracked-files=normal"
    };
    let text = reader.execute(
        &[
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--no-renames",
            "--ignore-submodules=all",
            mode,
        ],
        false,
    )?;
    let mut full = parse_status(&text)?;
    let entries = full["entries"]
        .as_array()
        .ok_or(("GIT_FORMAT", "Missing entries"))?;
    let mut counts = json!({"staged":0,"modified":0,"untracked":0,"conflicts":0});
    for entry in entries {
        let mut increment = |key: &str| {
            counts[key] = json!(counts[key].as_u64().unwrap_or(0) + 1);
        };
        match entry["kind"].as_str() {
            Some("untracked") => increment("untracked"),
            Some("conflict") => increment("conflicts"),
            _ => {
                let xy = entry["xy"].as_str().unwrap_or("..").as_bytes();
                if xy[0] != b'.' {
                    increment("staged");
                }
                if xy[1] != b'.' {
                    increment("modified");
                }
            }
        }
    }
    let total = entries.len();
    let mut selected = Vec::new();
    let mut used = 0;
    for entry in entries.iter().skip(offset).take(limit) {
        let size = utf16_len(&entry.to_string());
        if used + size > 6000 {
            if selected.is_empty() {
                return Err(("GIT_SIZE", "One status entry exceeds the response budget"));
            }
            break;
        }
        used += size;
        selected.push(entry.clone());
    }
    let next = (offset + selected.len() < total).then_some(offset + selected.len());
    let detail = full.clone();
    full["entries"] = json!(selected);
    full["counts"] = counts;
    full["total"] = json!(total);
    full["offset"] = json!(offset);
    full["nextOffset"] = json!(next);
    full["milliseconds"] = json!(started.elapsed().as_millis() as u64);
    full["scope"] = json!("workspace; submodules excluded; untracked directories grouped");
    if next.is_some() {
        full["truncated"] = json!(true);
        full["_detail"] = detail;
    }
    Ok(full)
}

pub(super) fn diff(
    project: &Project,
    args: &GitDiffArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
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
    let file = project.target_optional(&normalized)?;
    if file.exists() && !file.is_file() {
        return Err((
            "PATH",
            "Git diff requires one regular file path, not a directory",
        ));
    }
    let started = Instant::now();
    let reader = Reader::new(project, context)?;
    let mut args_list = vec!["diff"];
    if args.staged == Some(true) {
        args_list.push("--cached");
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
        &normalized,
    ]);
    let patch = reader.execute(&args_list, false)?;
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
    let truncated = lines.len() > lines_limit || utf16_len(&joined) > chars;
    let comparison = if args.staged == Some(true) {
        "HEAD:index"
    } else {
        "index:worktree"
    };
    let hunks: Vec<_> = lines
        .iter()
        .filter(|s| s.starts_with("@@"))
        .take(20)
        .map(|s| prefix(s, 300))
        .collect();
    let mut result = json!({"path":normalized,"comparison":comparison,"changed":!patch.is_empty(),"binary":lines.iter().any(|s|s.starts_with("Binary files ")),"hunks":hunks,"totalLines":lines.len(),"preview":preview,"milliseconds":started.elapsed().as_millis() as u64,
        "note":"Tracked changes only; textual diff, not semantic proof. Hunk labels come from Git."});
    if truncated {
        result["truncated"] = json!(true);
        result["_detail"] = json!({"path":normalized,"comparison":comparison,"patch":patch});
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_branch_spaces_conflict_and_rename_with_nul_delimiters() {
        let output = concat!(
            "# branch.oid (initial)\0# branch.head main\0# branch.ab +2 -3\0? a b\0u UU N... 100644 100644 100644 100644 a b c conflict\0",
            "2 R. N... 100644 100644 100644 a b R100 new name\0old name\0"
        );
        let result = parse_status(output).unwrap();
        assert_eq!(result["unborn"], true);
        assert_eq!(result["ahead"], 2);
        assert_eq!(result["behind"], 3);
        assert_eq!(result["entries"][0]["path"], "a b");
        assert_eq!(result["entries"][1]["kind"], "conflict");
        assert_eq!(result["entries"][2]["from"], "old name");
        assert!(parse_status("1 broken\0").is_err());
        assert!(parse_status("? truncated").is_err());
    }
}

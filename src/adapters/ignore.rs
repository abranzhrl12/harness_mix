//! Manage `.harnessignore` policy files with validation and atomic publication.
use super::project::Project;
use harness_domain::{
    Failure,
    contracts::{IgnoreAction, IgnoreArgs},
    execution::OperationContext,
    hash,
};
use serde_json::{Value, json};
use std::{fs, io::Write, path::Path};

pub(super) const FILE: &str = ".harnessignore";
pub(super) const MAX_PATTERNS: usize = 128;
const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_PATTERN_CHARS: usize = 1024;
pub(super) const DEFAULT_EXCLUSIONS: &[&str] = &["node_modules/", "target/"];
pub(super) const HARD_EXCLUSIONS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".env*",
    "credential*",
    "secret*",
    "id_rsa",
    "id_ed25519",
    "oauth_creds.json",
    "mcp_config.json",
];

pub(super) fn ignore(
    project: &Project,
    args: &IgnoreArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    let scope = normalize_scope(args.scope.as_deref())?;
    if scope != "."
        && scope.split('/').any(|part| {
            [".git", ".hg", ".svn", ".agents", ".codegraph"]
                .contains(&part.to_ascii_lowercase().as_str())
        })
    {
        return Err((
            "PATH",
            "Ignore policy cannot be managed in version-control or harness metadata directories",
        ));
    }
    let directory = project.target(&scope)?;
    let metadata = fs::metadata(&directory).map_err(|_| ("PATH", "Ignore scope unavailable"))?;
    if !metadata.is_dir() {
        return Err(("PATH", "Ignore scope must be a directory"));
    }
    let target = directory.join(FILE);
    if let Ok(metadata) = fs::symlink_metadata(&target) {
        if metadata.file_type().is_symlink() {
            return Err(("PATH", ".harnessignore cannot be a symbolic link"));
        }
    }
    match args.action {
        IgnoreAction::List => list(project, &scope, &target, context),
        IgnoreAction::Init => init(project, &scope, &target, context),
        IgnoreAction::Add => add(project, &scope, &target, args, context),
        IgnoreAction::Remove => remove(project, &scope, &target, args, context),
    }
}

fn normalize_scope(scope: Option<&str>) -> Result<String, Failure> {
    let scope = scope.unwrap_or(".");
    if scope.contains(['\n', '\r', '\0']) || scope.chars().any(char::is_control) {
        return Err(("ARGUMENT", "Ignore scope contains control characters"));
    }
    let scope = scope.trim();
    Ok(if scope.is_empty() || scope == "." {
        ".".to_owned()
    } else {
        scope.replace('\\', "/")
    })
}

fn relative(project: &Project, target: &Path) -> String {
    target
        .strip_prefix(&project.root)
        .unwrap_or(target)
        .to_string_lossy()
        .replace('\\', "/")
}

fn list(
    project: &Project,
    scope: &str,
    target: &Path,
    context: &OperationContext,
) -> Result<Value, Failure> {
    let mut sources = Vec::new();
    let mut inherited = Vec::new();
    let mut chain = vec![project.root.clone()];
    if scope != "." {
        let mut current = project.root.clone();
        for part in scope.split('/').filter(|part| !part.is_empty()) {
            current.push(part);
            chain.push(current.clone());
        }
    }
    for directory in chain {
        context.check()?;
        let file = directory.join(FILE);
        match read_policy(&file)? {
            Some((text, patterns, entries)) => {
                let source = json!({
                    "path": relative(project, &file),
                    "scope": relative(project, &directory),
                    "patterns": patterns,
                    "entries": entries,
                    "hash": hash(text.as_bytes()),
                });
                if directory == project.root {
                    sources.push(source);
                } else if directory == target.parent().unwrap_or(&project.root) {
                    sources.push(source);
                } else {
                    inherited.push(source);
                }
            }
            None => {}
        }
    }
    let managed = sources
        .last()
        .filter(|source| source["scope"] == json!(scope))
        .cloned();
    Ok(json!({
        "action":"list",
        "scope":scope,
        "path":relative(project, target),
        "exists":target.try_exists().unwrap_or(false),
        "managed":managed,
        "sources":sources,
        "inherited":inherited,
        "defaults":DEFAULT_EXCLUSIONS,
        "hardExclusions":HARD_EXCLUSIONS,
        "policy":"Patterns use gitignore syntax. Add with harness_ignore action=add; inspect before changing.",
        "limits":{"patterns":MAX_PATTERNS,"fileBytes":MAX_FILE_BYTES,"patternChars":MAX_PATTERN_CHARS},
    }))
}

fn init(
    project: &Project,
    scope: &str,
    target: &Path,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    if target.try_exists().unwrap_or(false) {
        let (text, patterns, entries) = read_policy(target)?
            .ok_or(("IO", "Ignore file exists but cannot be read as UTF-8 text"))?;
        return Ok(json!({
            "action":"init","scope":scope,"path":relative(project, target),
            "exists":true,"created":false,"changed":false,"unchanged":true,
            "patterns":patterns,"entries":entries,"hash":hash(text.as_bytes()),
            "defaults":DEFAULT_EXCLUSIONS,"hardExclusions":HARD_EXCLUSIONS,
        }));
    }
    context.check()?;
    let content = format!(
        "# Harness search exclusions (gitignore syntax).\n\
         # Managed with harness_ignore; generated files, caches and vendor output belong here.\n\
         # Secrets, VCS metadata and defaults cannot be re-included.\n\
         # Defaults: {}\n",
        DEFAULT_EXCLUSIONS.join(", ")
    );
    write_atomic(target, &content, false)?;
    Ok(json!({
        "action":"init","scope":scope,"path":relative(project, target),
        "exists":true,"created":true,"changed":true,"unchanged":false,
        "patterns":[],"entries":0,"hash":hash(content.as_bytes()),
        "defaults":DEFAULT_EXCLUSIONS,"hardExclusions":HARD_EXCLUSIONS,
    }))
}

fn add(
    project: &Project,
    scope: &str,
    target: &Path,
    args: &IgnoreArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    let patterns = requested_patterns(args)?;
    let reason = clean_reason(args.reason.as_deref())?;
    let existing = read_policy(target)?;
    let (mut text, _existing_patterns, _entries) = existing.unwrap_or_default();
    let present: std::collections::BTreeSet<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect();
    let mut added = Vec::new();
    for pattern in patterns {
        if present.contains(&pattern) {
            continue;
        }
        added.push(pattern);
    }
    if added.is_empty() {
        let entries = entries_of(&text);
        return Ok(json!({
            "action":"add","scope":scope,"path":relative(project, target),
            "exists":target.try_exists().unwrap_or(false),"created":false,
            "changed":false,"unchanged":true,"added":[],"removed":[],
            "patterns":entries.1,"entries":entries.0,
            "hash":if target.try_exists().unwrap_or(false){json!(hash(text.as_bytes()))}else{Value::Null},
            "defaults":DEFAULT_EXCLUSIONS,"hardExclusions":HARD_EXCLUSIONS,
        }));
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if let Some(reason) = reason {
        text.push_str("# ");
        text.push_str(&reason);
        text.push('\n');
    }
    for pattern in &added {
        text.push_str(pattern);
        text.push('\n');
    }
    let (entry_count, pattern_count) = entries_of(&text);
    if pattern_count > MAX_PATTERNS {
        return Err((
            "LIMIT",
            "Ignore policy would exceed 128 patterns; remove obsolete entries first",
        ));
    }
    let created = !target.try_exists().unwrap_or(false);
    context.check()?;
    write_atomic(target, &text, !created)?;
    Ok(json!({
        "action":"add","scope":scope,"path":relative(project, target),
        "exists":true,"created":created,"changed":true,"unchanged":false,
        "added":added,"removed":[],
        "patterns":pattern_count,"entries":entry_count,
        "hash":hash(text.as_bytes()),
        "defaults":DEFAULT_EXCLUSIONS,"hardExclusions":HARD_EXCLUSIONS,
        "effective":"Applies to harness_search and other ignore-aware workspace walks; secrets and VCS metadata remain excluded.",
    }))
}

fn remove(
    project: &Project,
    scope: &str,
    target: &Path,
    args: &IgnoreArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    let patterns = requested_patterns(args)?;
    let Some((text, _patterns, _entries)) = read_policy(target)? else {
        return Ok(json!({
            "action":"remove","scope":scope,"path":relative(project, target),
            "exists":false,"changed":false,"unchanged":true,
            "added":[],"removed":[],"patterns":0,"entries":0,"hash":Value::Null,
            "defaults":DEFAULT_EXCLUSIONS,"hardExclusions":HARD_EXCLUSIONS,
        }));
    };
    let wanted: std::collections::BTreeSet<&str> = patterns.iter().map(String::as_str).collect();
    let mut removed = Vec::new();
    let mut kept = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() && !trimmed.starts_with('#') && wanted.contains(trimmed) {
            removed.push(trimmed.to_owned());
        } else {
            kept.push(line);
        }
    }
    if removed.is_empty() {
        let (entries, pattern_count) = entries_of(&text);
        return Ok(json!({
            "action":"remove","scope":scope,"path":relative(project, target),
            "exists":true,"changed":false,"unchanged":true,
            "added":[],"removed":[],"patterns":pattern_count,"entries":entries,
            "hash":hash(text.as_bytes()),
            "defaults":DEFAULT_EXCLUSIONS,"hardExclusions":HARD_EXCLUSIONS,
        }));
    }
    let mut updated = kept.join("\n");
    if !updated.is_empty() {
        updated.push('\n');
    }
    let (entries, pattern_count) = entries_of(&updated);
    context.check()?;
    write_atomic(target, &updated, true)?;
    Ok(json!({
        "action":"remove","scope":scope,"path":relative(project, target),
        "exists":true,"changed":true,"unchanged":false,
        "added":[],"removed":removed,
        "patterns":pattern_count,"entries":entries,
        "hash":hash(updated.as_bytes()),
        "defaults":DEFAULT_EXCLUSIONS,"hardExclusions":HARD_EXCLUSIONS,
    }))
}

fn requested_patterns(args: &IgnoreArgs) -> Result<Vec<String>, Failure> {
    let patterns = args.patterns.as_deref().unwrap_or_default();
    if patterns.is_empty() {
        return Err((
            "ARGUMENT",
            "patterns requiere entre 1 y 128 patrones para add/remove",
        ));
    }
    if patterns.len() > MAX_PATTERNS {
        return Err(("LIMIT", "patterns admite como máximo 128 patrones"));
    }
    let mut result = Vec::with_capacity(patterns.len());
    for pattern in patterns {
        let pattern = pattern.trim();
        validate_pattern(pattern)?;
        if !result.iter().any(|existing: &String| existing == pattern) {
            result.push(pattern.to_owned());
        }
    }
    Ok(result)
}

fn clean_reason(reason: Option<&str>) -> Result<Option<String>, Failure> {
    let Some(reason) = reason else {
        return Ok(None);
    };
    let reason = reason.trim();
    if reason.is_empty() {
        return Ok(None);
    }
    if reason.chars().count() > 200
        || reason.contains(['\n', '\r', '\0'])
        || reason.chars().any(char::is_control)
    {
        return Err((
            "ARGUMENT",
            "reason debe ser texto de una línea, máximo 200 caracteres",
        ));
    }
    Ok(Some(reason.replace('\r', " ")))
}

fn validate_pattern(pattern: &str) -> Result<(), Failure> {
    if pattern.is_empty()
        || pattern.chars().count() > MAX_PATTERN_CHARS
        || pattern.contains(['\n', '\r', '\0'])
        || pattern.chars().any(char::is_control)
    {
        return Err((
            "ARGUMENT",
            "cada patrón debe ser no vacío, sin control/NUL ni saltos de línea, máximo 1024 caracteres",
        ));
    }
    let body = pattern.strip_prefix('!').unwrap_or(pattern);
    if body.is_empty() {
        return Err(("ARGUMENT", "A negation marker requires a pattern"));
    }
    let normalized = body.replace('\\', "/");
    if normalized.starts_with('/') || body.contains(':') {
        return Err((
            "ARGUMENT",
            "los patrones deben ser relativos al directorio de .harnessignore, sin rutas absolutas",
        ));
    }
    if normalized
        .split('/')
        .any(|part| part == ".." || part == "." && normalized.contains('/'))
    {
        return Err(("ARGUMENT", "los patrones no pueden salir del scope con .."));
    }
    if normalized
        .split('/')
        .any(|part| [".git", ".hg", ".svn"].contains(&part.to_ascii_lowercase().as_str()))
    {
        return Err((
            "ARGUMENT",
            "Version-control metadata cannot be excluded or re-included by policy",
        ));
    }
    let lower = normalized.to_ascii_lowercase();
    let reinclude = pattern.starts_with('!');
    let sensitive = lower.starts_with(".env")
        || lower.starts_with("credential")
        || lower.starts_with("secret")
        || [
            "id_rsa",
            "id_ed25519",
            "oauth_creds.json",
            "mcp_config.json",
        ]
        .contains(&lower.as_str());
    if reinclude && sensitive {
        return Err((
            "PATH",
            "Secret and credential paths cannot be re-included by policy",
        ));
    }
    if matches!(
        pattern,
        "*" | "**" | "/*" | "/**" | "!*" | "!**" | "!/*" | "!/**"
    ) || normalized == "**"
    {
        return Err((
            "ARGUMENT",
            "A pattern that excludes the entire workspace is not allowed",
        ));
    }
    Ok(())
}

fn entries_of(text: &str) -> (usize, usize) {
    let entries = text
        .lines()
        .filter(|line| {
            let line = line.trim();
            !line.is_empty() && !line.starts_with('#')
        })
        .count();
    (entries, entries)
}

fn read_policy(path: &Path) -> Result<Option<(String, usize, usize)>, Failure> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(("IO", "Cannot inspect .harnessignore")),
    };
    if !metadata.is_file() || metadata.len() as usize > MAX_FILE_BYTES {
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
    let (entries, patterns) = entries_of(&text);
    if patterns > MAX_PATTERNS {
        return Err((
            "LIMIT",
            "Existing .harnessignore exceeds 128 patterns; reduce it before managing it",
        ));
    }
    Ok(Some((text, patterns, entries)))
}

fn write_atomic(path: &Path, content: &str, replace_existing: bool) -> Result<(), Failure> {
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

/// Effective pattern lines for one policy file, used by search reporting.
pub(super) fn pattern_count(project: &Project, relative_scope: &str) -> Option<usize> {
    let directory = project.target(relative_scope).ok()?;
    let file = directory.join(FILE);
    read_policy(&file)
        .ok()
        .flatten()
        .map(|(_, patterns, _)| patterns)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(action: IgnoreAction, patterns: &[&str]) -> IgnoreArgs {
        IgnoreArgs {
            action,
            scope: None,
            patterns: Some(
                patterns
                    .iter()
                    .map(|pattern| (*pattern).to_owned())
                    .collect(),
            ),
            reason: None,
            session: None,
        }
    }

    #[test]
    fn add_is_idempotent_and_preserves_existing_entries() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(FILE), "# keep\n*.log\n").unwrap();
        let project = Project::new(dir.path()).unwrap();
        let mut request = args(IgnoreAction::Add, &["dist/", "*.log"]);
        request.reason = Some("generated output".into());
        let result = ignore(&project, &request, &OperationContext::default()).unwrap();
        assert_eq!(result["added"], json!(["dist/"]));
        assert_eq!(result["patterns"], 2);
        let text = fs::read_to_string(dir.path().join(FILE)).unwrap();
        assert!(text.contains("# keep\n*.log\n"));
        assert!(text.contains("# generated output\ndist/\n"));
        let again = ignore(&project, &request, &OperationContext::default()).unwrap();
        assert_eq!(again["unchanged"], true);
    }

    #[test]
    fn remove_and_init_are_explicit_and_safe() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project::new(dir.path()).unwrap();
        let init = ignore(
            &project,
            &args(IgnoreAction::Init, &[]),
            &OperationContext::default(),
        )
        .unwrap();
        assert_eq!(init["created"], true);
        assert!(
            fs::read_to_string(dir.path().join(FILE))
                .unwrap()
                .starts_with("# Harness")
        );
        ignore(
            &project,
            &args(IgnoreAction::Add, &["build/"]),
            &OperationContext::default(),
        )
        .unwrap();
        let removed = ignore(
            &project,
            &args(IgnoreAction::Remove, &["build/"]),
            &OperationContext::default(),
        )
        .unwrap();
        assert_eq!(removed["removed"], json!(["build/"]));
        assert_eq!(removed["patterns"], 0);
    }

    #[test]
    fn unsafe_patterns_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project::new(dir.path()).unwrap();
        for pattern in [
            "*",
            "../outside",
            "/absolute",
            "!.env",
            "!secret.txt",
            ".git/",
        ] {
            let error = ignore(
                &project,
                &args(IgnoreAction::Add, &[pattern]),
                &OperationContext::default(),
            )
            .unwrap_err();
            assert!(
                error.0 == "ARGUMENT" || error.0 == "PATH",
                "{pattern}: {error:?}"
            );
        }
    }
}

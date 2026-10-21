//! Manage `.harnessignore` policy files with validation and atomic publication.
use super::super::project::Project;
use super::{policy, storage};
use harness_domain::{
    Failure,
    contracts::{IgnoreAction, IgnoreArgs},
    execution::OperationContext,
    hash,
};
use serde_json::{Value, json};
use std::{fs, path::Path};

use super::{DEFAULT_EXCLUSIONS, FILE, HARD_EXCLUSIONS};

pub(super) fn ignore(
    project: &Project,
    args: &IgnoreArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    let scope = policy::normalize_scope(args.scope.as_deref())?;
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
        match storage::read_policy(&file)? {
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
        "limits":{"patterns":super::MAX_PATTERNS,"fileBytes":policy::MAX_FILE_BYTES,"patternChars":policy::MAX_PATTERN_CHARS},
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
        let (text, patterns, entries) = storage::read_policy(target)?
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
    storage::write_atomic(target, &content, false)?;
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
    let patterns = policy::requested_patterns(args)?;
    let reason = policy::clean_reason(args.reason.as_deref())?;
    let existing = storage::read_policy(target)?;
    let (text, _existing_patterns, _entries) = existing.unwrap_or_default();
    let outcome = policy::add_patterns(&text, &patterns, reason.as_deref())?;
    if outcome.added.is_empty() {
        let entries = policy::entries_of(&text);
        return Ok(json!({
            "action":"add","scope":scope,"path":relative(project, target),
            "exists":target.try_exists().unwrap_or(false),"created":false,
            "changed":false,"unchanged":true,"added":[],"removed":[],
            "patterns":entries.1,"entries":entries.0,
            "hash":if target.try_exists().unwrap_or(false){json!(hash(text.as_bytes()))}else{Value::Null},
            "defaults":DEFAULT_EXCLUSIONS,"hardExclusions":HARD_EXCLUSIONS,
        }));
    }
    let text = outcome.text;
    let (entry_count, pattern_count) = (outcome.entries, outcome.patterns);
    let created = !target.try_exists().unwrap_or(false);
    context.check()?;
    storage::write_atomic(target, &text, !created)?;
    Ok(json!({
        "action":"add","scope":scope,"path":relative(project, target),
        "exists":true,"created":created,"changed":true,"unchanged":false,
        "added":outcome.added,"removed":[],
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
    let patterns = policy::requested_patterns(args)?;
    let Some((text, _patterns, _entries)) = storage::read_policy(target)? else {
        return Ok(json!({
            "action":"remove","scope":scope,"path":relative(project, target),
            "exists":false,"changed":false,"unchanged":true,
            "added":[],"removed":[],"patterns":0,"entries":0,"hash":Value::Null,
            "defaults":DEFAULT_EXCLUSIONS,"hardExclusions":HARD_EXCLUSIONS,
        }));
    };
    let outcome = policy::remove_patterns(&text, &patterns);
    let removed = outcome.removed;
    if removed.is_empty() {
        let (entries, pattern_count) = policy::entries_of(&text);
        return Ok(json!({
            "action":"remove","scope":scope,"path":relative(project, target),
            "exists":true,"changed":false,"unchanged":true,
            "added":[],"removed":[],"patterns":pattern_count,"entries":entries,
            "hash":hash(text.as_bytes()),
            "defaults":DEFAULT_EXCLUSIONS,"hardExclusions":HARD_EXCLUSIONS,
        }));
    }
    let updated = outcome.text;
    let (entries, pattern_count) = (outcome.entries, outcome.patterns);
    context.check()?;
    storage::write_atomic(target, &updated, true)?;
    Ok(json!({
        "action":"remove","scope":scope,"path":relative(project, target),
        "exists":true,"changed":true,"unchanged":false,
        "added":[],"removed":removed,
        "patterns":pattern_count,"entries":entries,
        "hash":hash(updated.as_bytes()),
        "defaults":DEFAULT_EXCLUSIONS,"hardExclusions":HARD_EXCLUSIONS,
    }))
}

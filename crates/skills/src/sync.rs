use std::path::Path;

use crate::{
    catalog::SKILLS,
    lock::SyncLock,
    manifest::{FORMAT as MANIFEST_FORMAT, Manifest, PATH as MANIFEST_PATH, contract_version},
    preferences::{PATH as PREFERENCES_PATH, read_preferences, set_auto_sync},
    report::{Action, Report},
    storage::{checked_root, read_safe_file, validate_relative, write_safe_file},
};

#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    pub check: bool,
    pub dry_run: bool,
    pub enable_auto: bool,
    pub disable_auto: bool,
}

enum PlannedAction {
    Create,
    Update(String),
    Current,
    Conflict,
}

/// Synchronize the bundled skills. `check` and `dry_run` only produce a plan.
pub fn run(root: &Path, options: Options) -> Result<Report, String> {
    if options.check && (options.dry_run || options.enable_auto || options.disable_auto)
        || options.dry_run && (options.enable_auto || options.disable_auto)
        || options.enable_auto && options.disable_auto
    {
        return Err("skills sync modes are mutually exclusive".into());
    }
    if options.disable_auto
        && !read_preferences(root)?.is_some_and(|preferences| preferences.auto_sync)
    {
        let mut report = Report::new();
        report.add_action("preferences", "already-disabled", PREFERENCES_PATH);
        return Ok(report);
    }

    let _lock = if options.check || options.dry_run {
        None
    } else {
        Some(SyncLock::acquire(root)?)
    };
    if options.disable_auto {
        let changed = set_auto_sync(root, false)?;
        let mut report = Report::new();
        report.changes = usize::from(changed);
        report.actions.push(Action {
            skill: "preferences".into(),
            action: "disabled",
            path: PREFERENCES_PATH.into(),
        });
        return Ok(report);
    }

    let mut report = sync(root, options.check, options.dry_run)?;
    if options.enable_auto
        && report.conflicts == 0
        && report.errors == 0
        && set_auto_sync(root, true)?
    {
        report.changes += 1;
        report.actions.push(Action {
            skill: "preferences".into(),
            action: "enabled",
            path: PREFERENCES_PATH.into(),
        });
    }
    Ok(report)
}

/// Run the sync at project startup only when the user opted in.
pub fn sync_on_startup(root: &Path) -> Result<Option<Report>, String> {
    let Some(preferences) = read_preferences(root)? else {
        return Ok(None);
    };
    if !preferences.auto_sync {
        return Ok(None);
    }
    let preview = sync(root, true, false)?;
    if preview.changes == 0 {
        return if preview.conflicts == 0 {
            Ok(None)
        } else {
            Ok(Some(preview))
        };
    }
    let _lock = SyncLock::acquire(root)?;
    if !read_preferences(root)?.is_some_and(|preferences| preferences.auto_sync) {
        return Ok(None);
    }
    sync(root, false, false).map(Some)
}

/// Synchronize bundled skill files and their ownership manifest.
pub fn sync(root: &Path, check: bool, dry_run: bool) -> Result<Report, String> {
    let root = checked_root(root)?;
    let manifest_relative = Path::new(MANIFEST_PATH);
    let manifest_bytes = read_safe_file(&root, manifest_relative)?;
    let old_manifest = manifest_bytes
        .as_deref()
        .map(serde_json::from_slice::<Manifest>)
        .transpose()
        .map_err(|error| format!("invalid skills manifest: {error}"))?;
    if let Some(manifest) = &old_manifest {
        if manifest.format_version != MANIFEST_FORMAT {
            return Err(format!(
                "unsupported skills manifest format {}; this package supports {}",
                manifest.format_version, MANIFEST_FORMAT
            ));
        }
        for (path, digest) in &manifest.files {
            validate_relative(Path::new(path))?;
            if !valid_digest(digest) {
                return Err(format!("invalid hash for manifest entry {path}"));
            }
        }
    }

    let old_files = old_manifest.as_ref().map(|manifest| &manifest.files);
    let mut report = Report::new();
    let mut plan = Vec::new();
    let mut next_files = old_files.cloned().unwrap_or_default();
    for source in SKILLS {
        validate_relative(Path::new(source.relative_path))?;
        let expected = source.content.as_bytes();
        let expected_hash = harness_domain::hash(expected);
        let current = read_safe_file(&root, Path::new(source.relative_path))?;
        let old_hash = old_files.and_then(|files| files.get(source.relative_path));
        let action = match current {
            None => PlannedAction::Create,
            Some(ref bytes) if harness_domain::hash(bytes) == expected_hash => {
                PlannedAction::Current
            }
            Some(ref bytes)
                if old_hash.is_some_and(|digest| digest == &harness_domain::hash(bytes)) =>
            {
                PlannedAction::Update(harness_domain::hash(bytes))
            }
            Some(_) => PlannedAction::Conflict,
        };
        match &action {
            PlannedAction::Conflict => {
                report.conflicts += 1;
                report.actions.push(Action {
                    skill: source.id.into(),
                    action: "conflict",
                    path: source.relative_path.into(),
                });
            }
            PlannedAction::Create | PlannedAction::Update(_) => {
                report.changes += 1;
                report.actions.push(Action {
                    skill: source.id.into(),
                    action: if matches!(&action, PlannedAction::Create) {
                        "create"
                    } else {
                        "update"
                    },
                    path: source.relative_path.into(),
                });
            }
            PlannedAction::Current => report.actions.push(Action {
                skill: source.id.into(),
                action: "current",
                path: source.relative_path.into(),
            }),
        }
        next_files.insert(source.relative_path.into(), expected_hash);
        plan.push((source, action));
    }

    let next_manifest = Manifest {
        format_version: MANIFEST_FORMAT,
        package_version: env!("CARGO_PKG_VERSION").into(),
        contract_version: contract_version().into(),
        files: next_files,
    };
    let next_manifest_bytes = serde_json::to_vec_pretty(&next_manifest)
        .map_err(|error| format!("cannot encode skills manifest: {error}"))?;
    let manifest_changed = manifest_bytes.as_deref() != Some(next_manifest_bytes.as_slice());
    if manifest_changed {
        report.changes += 1;
        report.actions.push(Action {
            skill: "manifest".into(),
            action: "write",
            path: MANIFEST_PATH.into(),
        });
    }
    if check || dry_run {
        for action in &mut report.actions {
            action.action = match action.action {
                "create" => "would-create",
                "update" => "would-update",
                "write" => "would-write",
                other => other,
            };
        }
        return Ok(report);
    }

    let mut writes_succeeded = true;
    for (source, action) in plan {
        let expected_hash = match action {
            PlannedAction::Current | PlannedAction::Conflict => continue,
            PlannedAction::Create => None,
            PlannedAction::Update(hash) => Some(hash),
        };
        if let Err(error) = write_safe_file(
            &root,
            Path::new(source.relative_path),
            source.content.as_bytes(),
            expected_hash.as_deref(),
        ) {
            writes_succeeded = false;
            report.errors += 1;
            report.actions.push(Action {
                skill: source.id.into(),
                action: "error",
                path: format!("{}: {error}", source.relative_path),
            });
        }
    }
    if report.conflicts == 0 && writes_succeeded && manifest_changed {
        let expected_hash = manifest_bytes.as_deref().map(harness_domain::hash);
        write_safe_file(
            &root,
            manifest_relative,
            &next_manifest_bytes,
            expected_hash.as_deref(),
        )
        .map_err(|error| format!("cannot update skills manifest: {error}"))?;
    }
    Ok(report)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};

const MANIFEST_PATH: &str = ".agents/skills/.harness-manifest.json";
const PREFERENCES_PATH: &str = ".agents/harness-skills-sync.json";
const LOCK_PATH: &str = ".agents/.harness-skills-sync.lock";
const DATABASE_CONFIG_PATH: &str = ".agents/harness.config.json";
const MCP_EXAMPLE_PATH: &str = ".agents/harness-mcp.example.json";
const MANIFEST_FORMAT: u32 = 1;
const PREFERENCES_FORMAT: u32 = 1;
const MAX_MANAGED_FILE_SIZE: u64 = 1_048_576;

struct SkillSource {
    id: &'static str,
    relative_path: &'static str,
    content: &'static str,
}

const SKILLS: &[SkillSource] = &[
    SkillSource {
        id: "harness-read",
        relative_path: ".agents/skills/harness-read/SKILL.md",
        content: include_str!("../../.agents/skills/harness-read/SKILL.md"),
    },
    SkillSource {
        id: "harness-edit",
        relative_path: ".agents/skills/harness-edit/SKILL.md",
        content: include_str!("../../.agents/skills/harness-edit/SKILL.md"),
    },
    SkillSource {
        id: "harness-sql",
        relative_path: ".agents/skills/harness-sql/SKILL.md",
        content: include_str!("../../.agents/skills/harness-sql/SKILL.md"),
    },
    SkillSource {
        id: "harness-xlsx",
        relative_path: ".agents/skills/harness-xlsx/SKILL.md",
        content: include_str!("../../.agents/skills/harness-xlsx/SKILL.md"),
    },
    SkillSource {
        id: "harness-svg",
        relative_path: ".agents/skills/harness-svg/SKILL.md",
        content: include_str!("../../.agents/skills/harness-svg/SKILL.md"),
    },
    SkillSource {
        id: "harness-setup",
        relative_path: ".agents/skills/harness-setup/SKILL.md",
        content: include_str!("../../.agents/skills/harness-setup/SKILL.md"),
    },
];

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Options {
    pub check: bool,
    pub dry_run: bool,
    pub enable_auto: bool,
    pub disable_auto: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct Report {
    pub package_version: &'static str,
    pub contract_version: &'static str,
    pub changes: usize,
    pub conflicts: usize,
    pub errors: usize,
    pub actions: Vec<Action>,
}

#[derive(Debug, Serialize)]
pub(super) struct Action {
    pub skill: String,
    pub action: &'static str,
    pub path: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format_version: u32,
    package_version: String,
    contract_version: String,
    files: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Preferences {
    format_version: u32,
    auto_sync: bool,
}

#[derive(Clone)]
enum PlannedAction {
    Create,
    Update(String),
    Current,
    Conflict,
}

#[derive(Clone, Copy)]
enum InitFilePlan {
    Create,
    Current,
    Preserve,
    Conflict,
}

#[derive(Clone, Copy)]
enum AutoSyncPlan {
    Enable,
    Current,
    Conflict,
}

struct InitPlan {
    report: Report,
    database_config: InitFilePlan,
    mcp_example: InitFilePlan,
}

pub(super) fn run(root: &Path, options: Options) -> Result<Report, String> {
    if options.check && (options.dry_run || options.enable_auto || options.disable_auto)
        || options.dry_run && (options.enable_auto || options.disable_auto)
        || options.enable_auto && options.disable_auto
    {
        return Err("skills sync modes are mutually exclusive".into());
    }
    if options.disable_auto {
        let previously_enabled =
            read_preferences(root)?.is_some_and(|preferences| preferences.auto_sync);
        if !previously_enabled {
            return Ok(Report {
                package_version: env!("CARGO_PKG_VERSION"),
                contract_version: contract_version(),
                changes: 0,
                conflicts: 0,
                errors: 0,
                actions: vec![Action {
                    skill: "preferences".into(),
                    action: "already-disabled",
                    path: PREFERENCES_PATH.into(),
                }],
            });
        }
    }
    let _lock = if options.check || options.dry_run {
        None
    } else {
        Some(SyncLock::acquire(root)?)
    };
    if options.disable_auto {
        let changed = set_auto_sync(root, false)?;
        return Ok(Report {
            package_version: env!("CARGO_PKG_VERSION"),
            contract_version: contract_version(),
            changes: usize::from(changed),
            conflicts: 0,
            errors: 0,
            actions: vec![Action {
                skill: "preferences".into(),
                action: "disabled",
                path: PREFERENCES_PATH.into(),
            }],
        });
    }

    let mut report = sync_skills(root, options.check, options.dry_run)?;
    if options.enable_auto && report.conflicts == 0 && report.errors == 0 {
        if set_auto_sync(root, true)? {
            report.changes += 1;
            report.actions.push(Action {
                skill: "preferences".into(),
                action: "enabled",
                path: PREFERENCES_PATH.into(),
            });
        }
    }
    Ok(report)
}

pub(super) fn initialize_project(root: &Path, executable: &Path) -> Result<Report, String> {
    let root = checked_root(root)?;
    let executable = checked_executable(executable)?;
    let mut plan = make_init_plan(&root, &executable)?;
    if plan.report.conflicts > 0 {
        return Ok(plan.report);
    }

    let _lock = SyncLock::acquire(&root)?;
    plan = make_init_plan(&root, &executable)?;
    if plan.report.conflicts > 0 {
        return Ok(plan.report);
    }

    let mut report = match sync_skills(&root, false, false) {
        Ok(report) => report,
        Err(error) => {
            let mut report = empty_report();
            report.errors = 1;
            report.actions.push(Action {
                skill: "skills".into(),
                action: "error",
                path: error,
            });
            return Ok(report);
        }
    };
    if report.conflicts > 0 || report.errors > 0 {
        return Ok(report);
    }

    let database_config = serde_json::json!({"database":{"autoDetect":true}});
    let executable_text = executable
        .to_str()
        .ok_or("canonical executable path is not valid UTF-8")?;
    let root_text = root
        .to_str()
        .ok_or("canonical project path is not valid UTF-8")?;
    let mcp_example = serde_json::json!({
        "mcpServers": {
            "harness": {
                "command": executable_text,
                "args": ["--root", root_text]
            }
        }
    });

    let mut writes_succeeded = true;
    if matches!(plan.database_config, InitFilePlan::Create) {
        match write_new_json(&root, DATABASE_CONFIG_PATH, &database_config) {
            Ok(()) => add_action(
                &mut report,
                "database-config",
                "created",
                DATABASE_CONFIG_PATH,
            ),
            Err(error) => {
                writes_succeeded = false;
                add_error(&mut report, "database-config", DATABASE_CONFIG_PATH, error);
            }
        }
    } else {
        add_action(
            &mut report,
            "database-config",
            if matches!(plan.database_config, InitFilePlan::Preserve) {
                "preserved"
            } else {
                "current"
            },
            DATABASE_CONFIG_PATH,
        );
    }

    if matches!(plan.mcp_example, InitFilePlan::Create) {
        match write_new_json(&root, MCP_EXAMPLE_PATH, &mcp_example) {
            Ok(()) => add_action(&mut report, "mcp-example", "created", MCP_EXAMPLE_PATH),
            Err(error) => {
                writes_succeeded = false;
                add_error(&mut report, "mcp-example", MCP_EXAMPLE_PATH, error);
            }
        }
    } else {
        add_action(&mut report, "mcp-example", "current", MCP_EXAMPLE_PATH);
    }

    if writes_succeeded {
        match set_auto_sync(&root, true) {
            Ok(true) => add_action(&mut report, "preferences", "enabled", PREFERENCES_PATH),
            Ok(false) => add_action(&mut report, "preferences", "current", PREFERENCES_PATH),
            Err(error) => add_error(&mut report, "preferences", PREFERENCES_PATH, error),
        }
    }
    Ok(report)
}

fn make_init_plan(root: &Path, executable: &Path) -> Result<InitPlan, String> {
    let skill_preview = sync_skills(root, true, false)?;
    let database_config = serde_json::json!({"database":{"autoDetect":true}});
    let executable_text = executable
        .to_str()
        .ok_or("canonical executable path is not valid UTF-8")?;
    let root_text = root
        .to_str()
        .ok_or("canonical project path is not valid UTF-8")?;
    let mcp_example = serde_json::json!({
        "mcpServers": {
            "harness": {
                "command": executable_text,
                "args": ["--root", root_text]
            }
        }
    });
    let database_plan = inspect_init_json(root, DATABASE_CONFIG_PATH, &database_config, true);
    let mcp_plan = inspect_init_json(root, MCP_EXAMPLE_PATH, &mcp_example, false);
    let auto_plan = match read_preferences(root) {
        Ok(Some(preferences)) if preferences.auto_sync => AutoSyncPlan::Current,
        Ok(None | Some(_)) => AutoSyncPlan::Enable,
        Err(_) => AutoSyncPlan::Conflict,
    };

    let mut report = empty_report();
    report.conflicts = skill_preview.conflicts;
    for mut action in skill_preview.actions {
        if action.action.starts_with("would-") {
            action.action = "not-applied";
        }
        report.actions.push(action);
    }
    append_init_plan_action(
        &mut report,
        "database-config",
        DATABASE_CONFIG_PATH,
        database_plan,
    );
    append_init_plan_action(&mut report, "mcp-example", MCP_EXAMPLE_PATH, mcp_plan);
    match auto_plan {
        AutoSyncPlan::Enable => {
            report.changes += 1;
            report.actions.push(Action {
                skill: "preferences".into(),
                action: "would-enable",
                path: PREFERENCES_PATH.into(),
            });
        }
        AutoSyncPlan::Current => report.actions.push(Action {
            skill: "preferences".into(),
            action: "current",
            path: PREFERENCES_PATH.into(),
        }),
        AutoSyncPlan::Conflict => {
            report.conflicts += 1;
            report.actions.push(Action {
                skill: "preferences".into(),
                action: "conflict",
                path: PREFERENCES_PATH.into(),
            });
        }
    }
    if report.conflicts > 0 {
        report.changes = 0;
    }
    Ok(InitPlan {
        report,
        database_config: database_plan,
        mcp_example: mcp_plan,
    })
}

fn inspect_init_json(
    root: &Path,
    relative: &str,
    desired: &serde_json::Value,
    preserve_different_valid_json: bool,
) -> InitFilePlan {
    match read_safe_file(root, Path::new(relative)) {
        Ok(None) => InitFilePlan::Create,
        Ok(Some(bytes)) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(value) if value == *desired => InitFilePlan::Current,
            Ok(value) if preserve_different_valid_json && value.is_object() => {
                InitFilePlan::Preserve
            }
            _ => InitFilePlan::Conflict,
        },
        Err(_) => InitFilePlan::Conflict,
    }
}

fn append_init_plan_action(report: &mut Report, skill: &str, path: &str, plan: InitFilePlan) {
    let action = match plan {
        InitFilePlan::Create => {
            report.changes += 1;
            "not-applied"
        }
        InitFilePlan::Current => "current",
        InitFilePlan::Preserve => "preserved",
        InitFilePlan::Conflict => {
            report.conflicts += 1;
            "conflict"
        }
    };
    report.actions.push(Action {
        skill: skill.into(),
        action,
        path: path.into(),
    });
}

fn checked_executable(path: &Path) -> Result<PathBuf, String> {
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("cannot resolve current executable path: {error}"))?;
    let metadata = fs::symlink_metadata(&canonical)
        .map_err(|error| format!("cannot inspect current executable: {error}"))?;
    if is_link_or_reparse(&metadata) || !metadata.is_file() {
        return Err("current executable must resolve to a regular file".into());
    }
    Ok(canonical)
}

fn write_new_json(root: &Path, relative: &str, value: &serde_json::Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("cannot encode {relative}: {error}"))?;
    write_safe_file(root, Path::new(relative), &bytes, None)
}

fn empty_report() -> Report {
    Report {
        package_version: env!("CARGO_PKG_VERSION"),
        contract_version: contract_version(),
        changes: 0,
        conflicts: 0,
        errors: 0,
        actions: Vec::new(),
    }
}

fn add_action(report: &mut Report, skill: &str, action: &'static str, path: &str) {
    report.changes += usize::from(matches!(action, "created" | "enabled"));
    report.actions.push(Action {
        skill: skill.into(),
        action,
        path: path.into(),
    });
}

fn add_error(report: &mut Report, skill: &str, path: &str, error: String) {
    report.errors += 1;
    report.actions.push(Action {
        skill: skill.into(),
        action: "error",
        path: format!("{path}: {error}"),
    });
}

pub(super) fn sync_on_startup(root: &Path) -> Result<Option<Report>, String> {
    let Some(preferences) = read_preferences(root)? else {
        return Ok(None);
    };
    if !preferences.auto_sync {
        return Ok(None);
    }
    let preview = sync_skills(root, true, false)?;
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
    sync_skills(root, false, false).map(Some)
}

struct SyncLock {
    path: PathBuf,
}

impl SyncLock {
    fn acquire(root: &Path) -> Result<Self, String> {
        let root = checked_root(root)?;
        let relative = Path::new(LOCK_PATH);
        let parent = create_safe_parents(&root, relative)?;
        if safe_metadata(&root, relative)?.is_some() {
            return Err(format!(
                "another skill sync owns {}; if no sync process is active, remove the stale lock after checking",
                relative.display()
            ));
        }
        let path = parent.join(".harness-skills-sync.lock");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| format!("cannot acquire skill sync lock: {error}"))?;
        Ok(Self { path })
    }
}

impl Drop for SyncLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn sync_skills(root: &Path, check: bool, dry_run: bool) -> Result<Report, String> {
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
                "unsupported skills manifest format {}; this CLI supports {}",
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
    let mut report = Report {
        package_version: env!("CARGO_PKG_VERSION"),
        contract_version: contract_version(),
        changes: 0,
        conflicts: 0,
        errors: 0,
        actions: Vec::new(),
    };
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
        if matches!(&action, PlannedAction::Conflict) {
            report.conflicts += 1;
            report.actions.push(Action {
                skill: source.id.into(),
                action: "conflict",
                path: source.relative_path.into(),
            });
        } else if matches!(&action, PlannedAction::Create | PlannedAction::Update(_)) {
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
        } else {
            report.actions.push(Action {
                skill: source.id.into(),
                action: "current",
                path: source.relative_path.into(),
            });
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
            if action.action == "create" {
                action.action = "would-create";
            } else if action.action == "update" {
                action.action = "would-update";
            } else if action.action == "write" {
                action.action = "would-write";
            }
        }
        return Ok(report);
    }

    let mut writes_succeeded = true;
    for (source, action) in plan {
        match action {
            PlannedAction::Current | PlannedAction::Conflict => {}
            PlannedAction::Create => {
                match write_safe_file(
                    &root,
                    Path::new(source.relative_path),
                    source.content.as_bytes(),
                    None,
                ) {
                    Ok(()) => {}
                    Err(error) => {
                        writes_succeeded = false;
                        report.errors += 1;
                        report.actions.push(Action {
                            skill: source.id.into(),
                            action: "error",
                            path: format!("{}: {error}", source.relative_path),
                        });
                    }
                }
            }
            PlannedAction::Update(old_hash) => {
                match write_safe_file(
                    &root,
                    Path::new(source.relative_path),
                    source.content.as_bytes(),
                    Some(&old_hash),
                ) {
                    Ok(()) => {}
                    Err(error) => {
                        writes_succeeded = false;
                        report.errors += 1;
                        report.actions.push(Action {
                            skill: source.id.into(),
                            action: "error",
                            path: format!("{}: {error}", source.relative_path),
                        });
                    }
                }
            }
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

fn read_preferences(root: &Path) -> Result<Option<Preferences>, String> {
    let root = checked_root(root)?;
    let bytes = read_safe_file(&root, Path::new(PREFERENCES_PATH))?;
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    let preferences: Preferences = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid skills sync preferences: {error}"))?;
    if preferences.format_version != PREFERENCES_FORMAT {
        return Err(format!(
            "unsupported skills sync preferences format {}; this CLI supports {}",
            preferences.format_version, PREFERENCES_FORMAT
        ));
    }
    Ok(Some(preferences))
}

fn set_auto_sync(root: &Path, enabled: bool) -> Result<bool, String> {
    let root = checked_root(root)?;
    let relative = Path::new(PREFERENCES_PATH);
    let previous = read_preferences(&root)?;
    if previous
        .as_ref()
        .is_some_and(|preferences| preferences.auto_sync == enabled)
        || previous.is_none() && !enabled
    {
        return Ok(false);
    }
    let current = read_safe_file(&root, relative)?;
    let expected_hash = current.as_deref().map(harness_domain::hash);
    let bytes = serde_json::to_vec_pretty(&Preferences {
        format_version: PREFERENCES_FORMAT,
        auto_sync: enabled,
    })
    .map_err(|error| format!("cannot encode skills sync preferences: {error}"))?;
    write_safe_file(&root, relative, &bytes, expected_hash.as_deref())
        .map_err(|error| format!("cannot update skills sync preferences: {error}"))?;
    Ok(true)
}

fn checked_root(root: &Path) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|error| format!("workspace root is not accessible: {error}"))?;
    if is_link_or_reparse(&metadata) || !metadata.is_dir() {
        return Err("workspace root must be a real directory, not a symlink".into());
    }
    fs::canonicalize(root).map_err(|error| format!("cannot resolve workspace root: {error}"))
}

fn validate_relative(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("unsafe relative path: {}", path.display()));
    }
    Ok(())
}

fn safe_metadata(root: &Path, relative: &Path) -> Result<Option<fs::Metadata>, String> {
    validate_relative(relative)?;
    let mut current = root.to_path_buf();
    let components = relative.components().collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(name) = component else {
            return Err(format!("unsafe relative path: {}", relative.display()));
        };
        current.push(name);
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("cannot inspect {}: {error}", current.display())),
        };
        if is_link_or_reparse(&metadata) {
            return Err(format!(
                "refusing symlink path component: {}",
                current.display()
            ));
        }
        if index + 1 < components.len() && !metadata.is_dir() {
            return Err(format!(
                "path component is not a directory: {}",
                current.display()
            ));
        }
        if index + 1 == components.len() {
            return Ok(Some(metadata));
        }
    }
    Ok(None)
}

fn read_safe_file(root: &Path, relative: &Path) -> Result<Option<Vec<u8>>, String> {
    match safe_metadata(root, relative)? {
        None => Ok(None),
        Some(metadata) if !metadata.is_file() => {
            Err(format!("expected a regular file at {}", relative.display()))
        }
        Some(_) => read_no_follow(&root.join(relative))
            .map(Some)
            .map_err(|error| format!("cannot read {}: {error}", relative.display())),
    }
}

fn create_safe_parents(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    validate_relative(relative)?;
    let parent = relative
        .parent()
        .ok_or_else(|| format!("file path has no parent: {}", relative.display()))?;
    let mut current = root.to_path_buf();
    for component in parent.components() {
        let Component::Normal(name) = component else {
            return Err(format!("unsafe parent path: {}", relative.display()));
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if is_link_or_reparse(&metadata) || !metadata.is_dir() => {
                return Err(format!("unsafe directory component: {}", current.display()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&current)
                    .map_err(|error| format!("cannot create {}: {error}", current.display()))?;
            }
            Err(error) => return Err(format!("cannot inspect {}: {error}", current.display())),
        }
        let canonical = fs::canonicalize(&current)
            .map_err(|error| format!("cannot resolve {}: {error}", current.display()))?;
        if !canonical.starts_with(root) {
            return Err(format!(
                "directory escapes workspace root: {}",
                current.display()
            ));
        }
    }
    Ok(root.join(parent))
}

fn write_safe_file(
    root: &Path,
    relative: &Path,
    bytes: &[u8],
    expected_hash: Option<&str>,
) -> Result<(), String> {
    let parent = create_safe_parents(root, relative)?;
    let target = root.join(relative);
    let current = read_safe_file(root, relative)?;
    match (current.as_deref(), expected_hash) {
        (None, None) => {}
        (Some(current), Some(expected)) if harness_domain::hash(current) == expected => {}
        (None, Some(_)) | (Some(_), None) => {
            return Err(format!("concurrent file change at {}", relative.display()));
        }
        (Some(_), Some(_)) => {
            return Err(format!("concurrent file change at {}", relative.display()));
        }
    }

    let file_name = relative
        .file_name()
        .ok_or_else(|| format!("invalid destination path: {}", relative.display()))?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        file_name.to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    let write_result = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if safe_metadata(root, relative)
            .map_err(|_| io::Error::other("unsafe destination"))?
            .is_some()
        {
            let latest = read_no_follow(&target)?;
            match expected_hash {
                Some(expected) if harness_domain::hash(&latest) == expected => {}
                _ => {
                    return Err(io::Error::other(
                        "destination changed during synchronization",
                    ));
                }
            }
        } else if expected_hash.is_some() {
            return Err(io::Error::other(
                "destination disappeared during synchronization",
            ));
        }
        fs::rename(&temporary, &target)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result.map_err(|error| format!("cannot atomically write {}: {error}", relative.display()))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn contract_version() -> &'static str {
    harness_domain::tool_contract::manifest()["contractVersion"]
        .as_str()
        .unwrap_or("unknown")
}

fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn read_no_follow(path: &Path) -> io::Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if is_link_or_reparse(&metadata) || !metadata.is_file() {
        return Err(io::Error::other("refusing non-regular or reparse file"));
    }
    if metadata.len() > MAX_MANAGED_FILE_SIZE {
        return Err(io::Error::other(
            "managed file exceeds the 1 MiB safety limit",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_MANAGED_FILE_SIZE + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MANAGED_FILE_SIZE {
        return Err(io::Error::other(
            "managed file exceeds the 1 MiB safety limit",
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[cfg(unix)]
    fn create_directory_symlink(source: &Path, target: &Path) -> io::Result<()> {
        std::os::unix::fs::symlink(source, target)
    }

    #[cfg(windows)]
    fn create_directory_symlink(source: &Path, target: &Path) -> io::Result<()> {
        let output = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "New-Item -ItemType Junction -Path $env:HARNESS_TEST_LINK -Target $env:HARNESS_TEST_TARGET | Out-Null",
            ])
            .env("HARNESS_TEST_LINK", target)
            .env("HARNESS_TEST_TARGET", source)
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ))
        }
    }

    fn fixture_root() -> tempfile::TempDir {
        tempdir().expect("tempdir")
    }

    #[test]
    fn fresh_sync_and_repeat_are_idempotent() {
        let temp = fixture_root();
        let first = sync_skills(temp.path(), false, false).unwrap();
        assert_eq!(first.conflicts, 0);
        assert_eq!(first.errors, 0);
        assert_eq!(first.changes, SKILLS.len() + 1);
        let second = sync_skills(temp.path(), false, false).unwrap();
        assert_eq!(second.changes, 0);
        assert_eq!(second.conflicts, 0);
    }

    #[test]
    fn updates_only_a_file_matching_the_prior_manifest_hash() {
        let temp = fixture_root();
        sync_skills(temp.path(), false, false).unwrap();
        let source = &SKILLS[0];
        let file = temp.path().join(source.relative_path);
        fs::write(&file, "old managed content").unwrap();
        let mut manifest: Manifest =
            serde_json::from_slice(&fs::read(temp.path().join(MANIFEST_PATH)).unwrap()).unwrap();
        manifest.files.insert(
            source.relative_path.into(),
            harness_domain::hash(b"old managed content"),
        );
        fs::write(
            temp.path().join(MANIFEST_PATH),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let report = sync_skills(temp.path(), false, false).unwrap();
        assert_eq!(report.conflicts, 0);
        assert_eq!(fs::read(file).unwrap(), source.content.as_bytes());
    }

    #[test]
    fn preserves_unmanaged_or_locally_edited_files() {
        let temp = fixture_root();
        let source = &SKILLS[0];
        let file = temp.path().join(source.relative_path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "local edit").unwrap();
        let report = sync_skills(temp.path(), false, false).unwrap();
        assert_eq!(report.conflicts, 1);
        assert_eq!(fs::read(file).unwrap(), b"local edit");
    }

    #[test]
    fn preserves_edits_to_previously_managed_files() {
        let temp = fixture_root();
        sync_skills(temp.path(), false, false).unwrap();
        let source = &SKILLS[0];
        let file = temp.path().join(source.relative_path);
        fs::write(&file, "local edit after install").unwrap();
        let report = sync_skills(temp.path(), false, false).unwrap();
        assert_eq!(report.conflicts, 1);
        assert_eq!(fs::read(file).unwrap(), b"local edit after install");
    }

    #[test]
    fn check_and_dry_run_create_no_directories() {
        let temp = fixture_root();
        let check = sync_skills(temp.path(), true, false).unwrap();
        assert_eq!(check.changes, SKILLS.len() + 1);
        assert!(!temp.path().join(".agents").exists());
        let preview = sync_skills(temp.path(), false, true).unwrap();
        assert_eq!(preview.changes, SKILLS.len() + 1);
        assert!(!temp.path().join(".agents").exists());
    }

    #[test]
    fn malformed_manifest_fails_before_any_write() {
        let temp = fixture_root();
        let manifest = temp.path().join(MANIFEST_PATH);
        fs::create_dir_all(manifest.parent().unwrap()).unwrap();
        fs::write(&manifest, "not json").unwrap();
        assert!(sync_skills(temp.path(), false, false).is_err());
        assert_eq!(fs::read(&manifest).unwrap(), b"not json");
        assert!(!temp.path().join(".agents/skills/harness-read").exists());
    }

    #[test]
    fn rejects_symlinked_skill_parent() {
        let temp = fixture_root();
        let outside = temp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        let skills_dir = temp.path().join(".agents/skills");
        fs::create_dir_all(skills_dir.parent().unwrap()).unwrap();
        create_directory_symlink(&outside, &skills_dir).unwrap();
        assert!(sync_skills(temp.path(), false, false).is_err());
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    }

    #[test]
    fn rejects_a_symlink_as_the_workspace_root() {
        let temp = fixture_root();
        let outside = fixture_root();
        let root_link = temp.path().join("linked-workspace");
        create_directory_symlink(outside.path(), &root_link).unwrap();
        assert!(sync_skills(&root_link, false, false).is_err());
        assert!(!outside.path().join(".agents").exists());
    }

    #[test]
    fn rejects_parent_traversal() {
        assert!(validate_relative(Path::new("../outside/SKILL.md")).is_err());
        assert!(validate_relative(Path::new("/outside/SKILL.md")).is_err());
    }

    #[test]
    fn auto_sync_is_opt_in_and_disableable() {
        let temp = fixture_root();
        assert!(sync_on_startup(temp.path()).unwrap().is_none());
        run(
            temp.path(),
            Options {
                enable_auto: true,
                ..Options::default()
            },
        )
        .unwrap();
        assert!(sync_on_startup(temp.path()).unwrap().is_none());
        let first_skill = temp.path().join(SKILLS[0].relative_path);
        fs::remove_file(&first_skill).unwrap();
        assert!(sync_on_startup(temp.path()).unwrap().is_some());
        assert_eq!(
            fs::read(&first_skill).unwrap(),
            SKILLS[0].content.as_bytes()
        );
        run(
            temp.path(),
            Options {
                disable_auto: true,
                ..Options::default()
            },
        )
        .unwrap();
        assert!(sync_on_startup(temp.path()).unwrap().is_none());
    }

    #[test]
    fn auto_sync_reports_conflicts_without_replacing_local_content() {
        let temp = fixture_root();
        run(
            temp.path(),
            Options {
                enable_auto: true,
                ..Options::default()
            },
        )
        .unwrap();
        let source = &SKILLS[0];
        let file = temp.path().join(source.relative_path);
        fs::write(&file, "keep this local edit").unwrap();
        let report = sync_on_startup(temp.path()).unwrap().unwrap();
        assert_eq!(report.conflicts, 1);
        assert_eq!(report.errors, 0);
        assert_eq!(fs::read(file).unwrap(), b"keep this local edit");
    }

    #[test]
    fn init_creates_project_files_with_canonical_paths_and_is_idempotent() {
        let temp = fixture_root();
        let project = temp.path().join("project with spaces ü");
        fs::create_dir_all(&project).unwrap();
        let executable = std::env::current_exe().unwrap();
        let canonical_project = fs::canonicalize(&project).unwrap();
        let canonical_executable = fs::canonicalize(&executable).unwrap();

        let first = initialize_project(&project, &executable).unwrap();
        assert_eq!(first.conflicts, 0);
        assert_eq!(first.errors, 0);
        assert!(first.changes > SKILLS.len());
        assert!(!project.join(".codegraph").exists());

        let database: serde_json::Value =
            serde_json::from_slice(&fs::read(project.join(DATABASE_CONFIG_PATH)).unwrap()).unwrap();
        assert_eq!(database["database"]["autoDetect"], true);
        let snippet: serde_json::Value =
            serde_json::from_slice(&fs::read(project.join(MCP_EXAMPLE_PATH)).unwrap()).unwrap();
        assert_eq!(
            snippet["mcpServers"]["harness"]["command"],
            canonical_executable.to_str().unwrap()
        );
        assert_eq!(
            snippet["mcpServers"]["harness"]["args"],
            serde_json::json!(["--root", canonical_project.to_str().unwrap()])
        );
        assert!(
            read_preferences(&project)
                .unwrap()
                .is_some_and(|preferences| preferences.auto_sync)
        );

        let second = initialize_project(&project, &executable).unwrap();
        assert_eq!(second.changes, 0);
        assert_eq!(second.conflicts, 0);
        assert_eq!(second.errors, 0);
    }

    #[test]
    fn init_preserves_a_valid_custom_database_config() {
        let temp = fixture_root();
        let project = temp.path();
        let config = project.join(DATABASE_CONFIG_PATH);
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        let custom = br#"{"postgresql":{"enabled":true,"connection":{"source":"environment","name":"APP_DATABASE_URL"}}}"#;
        fs::write(&config, custom).unwrap();

        let report = initialize_project(project, &std::env::current_exe().unwrap()).unwrap();
        assert_eq!(report.conflicts, 0);
        assert_eq!(report.errors, 0);
        assert_eq!(fs::read(config).unwrap(), custom);
        assert!(
            report.actions.iter().any(|action| {
                action.path == DATABASE_CONFIG_PATH && action.action == "preserved"
            })
        );
    }

    #[test]
    fn init_snippet_conflict_aborts_before_other_project_writes() {
        let temp = fixture_root();
        let project = temp.path();
        let snippet = project.join(MCP_EXAMPLE_PATH);
        fs::create_dir_all(snippet.parent().unwrap()).unwrap();
        let local = br#"{"mcpServers":{"other":{"command":"local-server","args":[]}}}"#;
        fs::write(&snippet, local).unwrap();

        let report = initialize_project(project, &std::env::current_exe().unwrap()).unwrap();
        assert_eq!(report.conflicts, 1);
        assert_eq!(report.changes, 0);
        assert_eq!(fs::read(snippet).unwrap(), local);
        assert!(!project.join(DATABASE_CONFIG_PATH).exists());
        assert!(!project.join(MANIFEST_PATH).exists());
        assert!(!project.join(PREFERENCES_PATH).exists());
    }

    #[test]
    fn init_malformed_database_config_conflicts_without_overwriting() {
        let temp = fixture_root();
        let project = temp.path();
        let config = project.join(DATABASE_CONFIG_PATH);
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, b"{broken json").unwrap();

        let report = initialize_project(project, &std::env::current_exe().unwrap()).unwrap();
        assert_eq!(report.conflicts, 1);
        assert_eq!(report.changes, 0);
        assert_eq!(fs::read(config).unwrap(), b"{broken json");
        assert!(!project.join(MANIFEST_PATH).exists());
        assert!(!project.join(MCP_EXAMPLE_PATH).exists());
        assert!(!project.join(PREFERENCES_PATH).exists());
    }

    #[test]
    fn init_rejects_reparse_point_for_agents_directory() {
        let temp = fixture_root();
        let outside = temp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        create_directory_symlink(&outside, &project.join(".agents")).unwrap();

        assert!(initialize_project(&project, &std::env::current_exe().unwrap()).is_err());
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    }
}

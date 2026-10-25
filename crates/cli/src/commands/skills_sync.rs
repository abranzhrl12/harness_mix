use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) use harness_skills::{Action, Options, Report};

const MANIFEST_PATH: &str = ".agents/skills/.harness-manifest.json";
const PREFERENCES_PATH: &str = ".agents/harness-skills-sync.json";
const DATABASE_CONFIG_PATH: &str = ".agents/harness.config.json";
const MCP_EXAMPLE_PATH: &str = ".agents/harness-mcp.example.json";

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
    harness_skills::run(root, options)
}

pub(super) fn sync_on_startup(root: &Path) -> Result<Option<Report>, String> {
    harness_skills::sync_on_startup(root)
}

pub(super) fn initialize_project(root: &Path, executable: &Path) -> Result<Report, String> {
    let root = harness_skills::project_support::ProjectSupport::checked_root(root)?;
    let executable = checked_executable(executable)?;
    let mut plan = make_init_plan(&root, &executable)?;
    if plan.report.conflicts > 0 {
        return Ok(plan.report);
    }

    let support = harness_skills::project_support::ProjectSupport::lock(&root)?;
    plan = make_init_plan(&root, &executable)?;
    if plan.report.conflicts > 0 {
        return Ok(plan.report);
    }

    let mut report = match support.sync() {
        Ok(report) => report,
        Err(error) => {
            let mut report = Report::new();
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
        "mcpServers": { "harness": { "command": executable_text, "args": ["--root", root_text] } }
    });

    let mut writes_succeeded = true;
    if matches!(plan.database_config, InitFilePlan::Create) {
        match support.write_new_json(DATABASE_CONFIG_PATH, &database_config) {
            Ok(()) => report.add_action("database-config", "created", DATABASE_CONFIG_PATH),
            Err(error) => {
                writes_succeeded = false;
                report.add_error("database-config", DATABASE_CONFIG_PATH, error);
            }
        }
    } else {
        report.add_action(
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
        match support.write_new_json(MCP_EXAMPLE_PATH, &mcp_example) {
            Ok(()) => report.add_action("mcp-example", "created", MCP_EXAMPLE_PATH),
            Err(error) => {
                writes_succeeded = false;
                report.add_error("mcp-example", MCP_EXAMPLE_PATH, error);
            }
        }
    } else {
        report.add_action("mcp-example", "current", MCP_EXAMPLE_PATH);
    }
    if writes_succeeded {
        match support.set_auto_sync(true) {
            Ok(true) => report.add_action("preferences", "enabled", PREFERENCES_PATH),
            Ok(false) => report.add_action("preferences", "current", PREFERENCES_PATH),
            Err(error) => report.add_error("preferences", PREFERENCES_PATH, error),
        }
    }
    Ok(report)
}

fn make_init_plan(root: &Path, executable: &Path) -> Result<InitPlan, String> {
    let skill_preview = harness_skills::project_support::ProjectSupport::preview(root)?;
    let database_config = serde_json::json!({"database":{"autoDetect":true}});
    let executable_text = executable
        .to_str()
        .ok_or("canonical executable path is not valid UTF-8")?;
    let root_text = root
        .to_str()
        .ok_or("canonical project path is not valid UTF-8")?;
    let mcp_example = serde_json::json!({
        "mcpServers": { "harness": { "command": executable_text, "args": ["--root", root_text] } }
    });
    let database_plan = inspect_init_json(root, DATABASE_CONFIG_PATH, &database_config, true);
    let mcp_plan = inspect_init_json(root, MCP_EXAMPLE_PATH, &mcp_example, false);
    let auto_plan = match harness_skills::project_support::ProjectSupport::preferences(root) {
        Ok(Some(preferences)) if preferences.auto_sync => AutoSyncPlan::Current,
        Ok(None | Some(_)) => AutoSyncPlan::Enable,
        Err(_) => AutoSyncPlan::Conflict,
    };

    let mut report = Report::new();
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
    match harness_skills::project_support::ProjectSupport::read_file(root, Path::new(relative)) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use tempfile::tempdir;

    #[cfg(unix)]
    fn create_directory_symlink(source: &Path, target: &Path) -> io::Result<()> {
        std::os::unix::fs::symlink(source, target)
    }

    #[cfg(windows)]
    fn create_directory_symlink(source: &Path, target: &Path) -> io::Result<()> {
        let output = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", "New-Item -ItemType Junction -Path $env:HARNESS_TEST_LINK -Target $env:HARNESS_TEST_TARGET | Out-Null"])
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

    #[test]
    fn init_creates_project_files_with_canonical_paths_and_is_idempotent() {
        let temp = tempdir().unwrap();
        let project = temp.path().join("project with spaces ü");
        fs::create_dir_all(&project).unwrap();
        let executable = std::env::current_exe().unwrap();
        let canonical_project = fs::canonicalize(&project).unwrap();
        let canonical_executable = fs::canonicalize(&executable).unwrap();
        let first = initialize_project(&project, &executable).unwrap();
        assert_eq!(first.conflicts, 0);
        assert_eq!(first.errors, 0);
        assert!(first.changes > 0);
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
            harness_skills::project_support::ProjectSupport::preferences(&project)
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
        let temp = tempdir().unwrap();
        let config = temp.path().join(DATABASE_CONFIG_PATH);
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        let custom = br#"{"postgresql":{"enabled":true,"connection":{"source":"environment","name":"APP_DATABASE_URL"}}}"#;
        fs::write(&config, custom).unwrap();
        let report = initialize_project(temp.path(), &std::env::current_exe().unwrap()).unwrap();
        assert_eq!(report.conflicts, 0);
        assert_eq!(report.errors, 0);
        assert_eq!(fs::read(config).unwrap(), custom);
        assert!(
            report
                .actions
                .iter()
                .any(|action| action.path == DATABASE_CONFIG_PATH && action.action == "preserved")
        );
    }

    #[test]
    fn init_snippet_conflict_aborts_before_other_project_writes() {
        let temp = tempdir().unwrap();
        let snippet = temp.path().join(MCP_EXAMPLE_PATH);
        fs::create_dir_all(snippet.parent().unwrap()).unwrap();
        let local = br#"{"mcpServers":{"other":{"command":"local-server","args":[]}}}"#;
        fs::write(&snippet, local).unwrap();
        let report = initialize_project(temp.path(), &std::env::current_exe().unwrap()).unwrap();
        assert_eq!(report.conflicts, 1);
        assert_eq!(report.changes, 0);
        assert_eq!(fs::read(snippet).unwrap(), local);
        assert!(!temp.path().join(DATABASE_CONFIG_PATH).exists());
        assert!(!temp.path().join(MANIFEST_PATH).exists());
        assert!(!temp.path().join(PREFERENCES_PATH).exists());
    }

    #[test]
    fn init_malformed_database_config_conflicts_without_overwriting() {
        let temp = tempdir().unwrap();
        let config = temp.path().join(DATABASE_CONFIG_PATH);
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, b"{broken json").unwrap();
        let report = initialize_project(temp.path(), &std::env::current_exe().unwrap()).unwrap();
        assert_eq!(report.conflicts, 1);
        assert_eq!(report.changes, 0);
        assert_eq!(fs::read(config).unwrap(), b"{broken json");
        assert!(!temp.path().join(MANIFEST_PATH).exists());
        assert!(!temp.path().join(MCP_EXAMPLE_PATH).exists());
        assert!(!temp.path().join(PREFERENCES_PATH).exists());
    }

    #[test]
    fn init_rejects_reparse_point_for_agents_directory() {
        let temp = tempdir().unwrap();
        let outside = temp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        let project = temp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        create_directory_symlink(&outside, &project.join(".agents")).unwrap();
        assert!(initialize_project(&project, &std::env::current_exe().unwrap()).is_err());
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    }
}

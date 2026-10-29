use std::{ffi::OsString, path::PathBuf};

use harness_cli::{Command, Root, needs_workspace_root, parse};

fn parse_args(args: &[&str]) -> Result<Command, String> {
    parse(args.iter().map(OsString::from).collect())
}

#[test]
fn parses_agent_menu_without_changing_stdio_server_default() {
    assert!(matches!(
        parse_args(&["active"]),
        Ok(Command::Active {
            root: Root::DetectOrCurrent,
            provider: None,
        })
    ));
    for args in [
        vec!["active", "antigravity", "--root", "project with spaces"],
        vec!["--root", "project with spaces", "active", "antigravity"],
    ] {
        assert!(matches!(parse_args(&args), Ok(Command::Active {
            root: Root::Explicit(root), provider: Some(provider),
        }) if root == PathBuf::from("project with spaces") && provider == "antigravity"));
    }
    for args in [vec!["config"], vec!["/config"]] {
        assert!(matches!(parse_args(&args), Ok(Command::Config)));
    }
    for args in [
        vec!["active", "unknown"],
        vec!["active", "--root"],
        vec!["active", "--root", ""],
        vec!["active", "antigravity", "antigravity"],
        vec!["--root", "one", "active", "--root", "two"],
        vec!["config", "unexpected"],
    ] {
        assert!(
            parse_args(&args).is_err(),
            "accepted invalid args: {args:?}"
        );
    }
}

#[test]
fn parses_server_defaults_and_flags() {
    assert!(matches!(
        parse_args(&[]),
        Ok(Command::Serve {
            persist_logs: false,
            ..
        })
    ));
    assert!(matches!(
        parse_args(&["--root", "workspace"]),
        Ok(Command::Serve {
            persist_logs: false,
            ..
        })
    ));
    assert!(matches!(
        parse_args(&["--root", "workspace", "--persist-logs"]),
        Ok(Command::Serve {
            persist_logs: true,
            ..
        })
    ));
    match parse_args(&["--root", "workspace"]).unwrap() {
        Command::Serve {
            root: Root::Explicit(root),
            ..
        } => assert_eq!(root, PathBuf::from("workspace")),
        _ => panic!("expected server command with explicit root"),
    }
}

#[test]
fn parses_workspace_manifest_server_option() {
    match parse_args(&["--workspace-config", "projects.json"]).unwrap() {
        Command::Serve {
            workspace_config: Some(path),
            persist_logs: false,
            ..
        } => {
            assert_eq!(path, PathBuf::from("projects.json"));
        }
        _ => panic!("expected workspace-config server command"),
    }
    assert!(matches!(
        parse_args(&["--workspace-config", "projects.json", "--persist-logs"]),
        Ok(Command::Serve {
            workspace_config: Some(_),
            persist_logs: true,
            ..
        })
    ));
}

#[test]
fn parses_explicit_project_index_without_changing_maintenance_commands() {
    assert!(matches!(parse_args(&["index"]), Ok(Command::IndexProject { root: Root::DetectOrCurrent })));
    for args in [vec!["index", "--root", "project"], vec!["--root", "project", "index"]] {
        assert!(matches!(parse_args(&args), Ok(Command::IndexProject { root: Root::Explicit(root) }) if root == PathBuf::from("project")));
    }
    assert!(parse_args(&["index", "unexpected"]).is_err());
}

#[test]
fn parses_index_aliases_and_unified_actions() {
    for args in [
        vec!["--root", "w", "--init-index"],
        vec!["--root", "w", "--init-index", "legacy-ignored"],
        vec!["--init-index"],
        vec!["--root", "w", "--index", "init"],
    ] {
        assert!(matches!(parse_args(&args), Ok(Command::InitIndex { .. })));
    }
    for args in [
        vec!["--root", "w", "--sync"],
        vec!["--root", "w", "--sync", "legacy-ignored"],
        vec!["--sync"],
        vec!["--root", "w", "--index", "sync"],
    ] {
        assert!(matches!(
            parse_args(&args),
            Ok(Command::SyncWorkspace { .. })
        ));
    }
    for args in [
        vec!["--root", "w", "--reindex"],
        vec!["--root", "w", "--reindex", "legacy-ignored"],
        vec!["--reindex"],
        vec!["--root", "w", "--index", "rebuild"],
    ] {
        assert!(matches!(parse_args(&args), Ok(Command::Rebuild { .. })));
    }
    assert!(matches!(
        parse_args(&["--root", "w", "--sync-plan"]),
        Ok(Command::Plan { .. })
    ));
    assert!(matches!(
        parse_args(&["--root", "w", "--sync-plan", "legacy-ignored"]),
        Ok(Command::Plan { .. })
    ));
    assert!(matches!(
        parse_args(&["--root", "w", "--index", "plan"]),
        Ok(Command::Plan { .. })
    ));
}

#[test]
fn parses_file_sync_options() {
    assert!(matches!(
        parse_args(&["--root", "w", "--sync-file", "a.rs"]),
        Ok(Command::SyncFile {
            force: false,
            config: None,
            ..
        })
    ));
    assert!(matches!(
        parse_args(&["--root", "w", "--sync-file", "a.rs", "--force"]),
        Ok(Command::SyncFile { force: true, .. })
    ));
    assert!(matches!(
        parse_args(&[
            "--root",
            "w",
            "--sync-file",
            "a.ts",
            "--tsconfig",
            "tsconfig.json"
        ]),
        Ok(Command::SyncFile {
            config: Some(_),
            ..
        })
    ));
    match parse_args(&[
        "--root",
        "workspace",
        "--sync-file",
        "src/app.ts",
        "--tsconfig",
        "tsconfig.json",
    ])
    .unwrap()
    {
        Command::SyncFile {
            root: Root::Explicit(root),
            relative,
            config: Some(config),
            force,
        } => {
            assert_eq!(root, PathBuf::from("workspace"));
            assert_eq!(relative, OsString::from("src/app.ts"));
            assert_eq!(config, OsString::from("tsconfig.json"));
            assert!(!force);
        }
        _ => panic!("expected configured file sync command"),
    }
    assert!(matches!(
        parse_args(&["--sync-file", "a.rs"]),
        Ok(Command::SyncFile {
            root: Root::Detect,
            ..
        })
    ));
}

#[test]
fn parses_skill_sync_modes_and_root_forms() {
    assert!(matches!(
        parse_args(&["skills", "sync"]),
        Ok(Command::SkillsSync {
            root: Root::Detect,
            check: false,
            dry_run: false,
            ..
        })
    ));
    assert!(matches!(
        parse_args(&["skills", "sync", "--check"]),
        Ok(Command::SkillsSync { check: true, .. })
    ));
    assert!(matches!(
        parse_args(&["--root", "workspace", "skills", "sync", "--enable-auto"]),
        Ok(Command::SkillsSync {
            root: Root::Explicit(root),
            enable_auto: true,
            ..
        }) if root == PathBuf::from("workspace")
    ));
    assert!(matches!(
        parse_args(&["skills", "sync", "--root", "workspace", "--dry-run"]),
        Ok(Command::SkillsSync {
            root: Root::Explicit(root),
            dry_run: true,
            ..
        }) if root == PathBuf::from("workspace")
    ));
    assert!(parse_args(&["wrong", "workspace", "skills", "sync"]).is_err());
    assert!(parse_args(&["--root", "workspace", "skills", "sync", "--root", "other"]).is_err());
    assert!(parse_args(&["skills", "sync", "--check", "--enable-auto"]).is_err());
    assert!(parse_args(&["skills", "sync", "--enable-auto", "--disable-auto"]).is_err());
    assert!(needs_workspace_root(&[OsString::from("skills")]));
}

#[test]
fn parses_project_init_with_an_explicit_root_in_either_position() {
    for args in [
        vec!["init", "--root", "workspace"],
        vec!["--root", "workspace", "init"],
    ] {
        assert!(matches!(
            parse_args(&args),
            Ok(Command::InitProject { root:Root::Explicit(root) }) if root == PathBuf::from("workspace")
        ));
    }
    assert!(matches!(
        parse_args(&["init"]),
        Ok(Command::InitProject {
            root: Root::DetectOrCurrent
        })
    ));
    assert!(parse_args(&["init", "--root", ""]).is_err());
    assert!(parse_args(&["--root", "", "init"]).is_err());
    assert!(parse_args(&["init", "--root", "w", "--root", "other"]).is_err());
    assert!(parse_args(&["--root", "w", "init", "--root", "other"]).is_err());
}

#[test]
fn summary_flag_is_restricted_to_plan_actions_and_keeps_default_full_mode() {
    assert!(matches!(
        parse_args(&["--root", "workspace", "--index", "plan"]),
        Ok(Command::Plan { summary: false, .. })
    ));
    assert!(matches!(
        parse_args(&["--root", "workspace", "--index", "plan", "--summary"]),
        Ok(Command::Plan { summary: true, .. })
    ));
    assert!(matches!(
        parse_args(&["--root", "workspace", "--sync-plan", "--summary"]),
        Ok(Command::Plan { summary: true, .. })
    ));
    assert!(matches!(
        parse_args(&["--index", "plan", "--summary"]),
        Ok(Command::Plan { summary: true, .. })
    ));
    for args in [
        vec!["--root", "w", "--sync", "--summary"],
        vec!["--root", "w", "--reindex", "--summary"],
        vec!["--root", "w", "--init-index", "--summary"],
        vec!["--root", "w", "--index", "sync", "--summary"],
        vec!["--root", "w", "--index", "rebuild", "--summary"],
        vec!["--root", "w", "--sync-file", "a.rs", "--summary"],
        vec!["--root", "w", "--persist-logs", "--summary"],
        vec!["--summary"],
    ] {
        assert!(parse_args(&args).is_err(), "unexpectedly accepted {args:?}");
    }
    assert!(matches!(
        parse_args(&["--root", "workspace", "--sync-plan", "legacy-ignored"]),
        Ok(Command::Plan { summary: false, .. })
    ));
}

#[cfg(windows)]
#[test]
fn preserves_non_utf8_windows_paths_during_parsing() {
    use std::os::windows::ffi::OsStringExt;

    let root = OsString::from_wide(&[0xD800]);
    let relative = OsString::from_wide(&[0xDC00]);
    match parse(vec![
        OsString::from("--root"),
        root.clone(),
        OsString::from("--sync-file"),
        relative.clone(),
    ])
    .unwrap()
    {
        Command::SyncFile {
            root: Root::Explicit(parsed_root),
            relative: parsed_relative,
            ..
        } => {
            assert_eq!(parsed_root.as_os_str(), root.as_os_str());
            assert_eq!(parsed_relative, relative);
        }
        _ => panic!("expected file sync command"),
    }
}

#[test]
fn preserves_help_version_and_parse_errors() {
    assert!(matches!(parse_args(&["--help"]), Ok(Command::Help)));
    assert!(matches!(parse_args(&["-h"]), Ok(Command::Help)));
    assert!(matches!(parse_args(&["--version"]), Ok(Command::Version)));
    assert_eq!(
        parse_args(&["--root", "w", "--bogus"]).err().unwrap(),
        "usage: agent-harness [--root <workspace>] [--workspace-config <manifest.json>] [--index <init|sync|rebuild|plan> | --persist-logs | --sync-file <relative> [--tsconfig <relative.json>]] | --version"
    );
    assert_eq!(
        parse_args(&["--root", "w", "a", "b", "c", "d"])
            .err()
            .unwrap(),
        "too many command-line arguments"
    );
}

#[test]
fn uninstall_accepts_current_project_and_explicit_root_without_extra_flags() {
    assert!(matches!(
        parse_args(&["uninstall"]),
        Ok(Command::UninstallProject {
            root: Root::DetectOrCurrent
        })
    ));
    for args in [
        vec!["uninstall", "--root", "workspace"],
        vec!["--root", "workspace", "uninstall"],
    ] {
        assert!(
            matches!(parse_args(&args),Ok(Command::UninstallProject {root:Root::Explicit(path)}) if path==PathBuf::from("workspace"))
        );
    }
    for args in [
        vec!["uninstall", "--root"],
        vec!["uninstall", "--root", ""],
        vec!["uninstall", "--force"],
        vec!["uninstall", "--root", "one", "--root", "two"],
    ] {
        assert!(parse_args(&args).is_err(), "{args:?}");
    }
}

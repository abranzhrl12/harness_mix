use std::process::Command;

fn command(root: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_agent-harness"));
    command.current_dir(root);
    for variable in [
        "WORKSPACE_ROOT",
        "VSCODE_CWD",
        "INIT_CWD",
        "PROJECT_ROOT",
        "ANTIGRAVITY_WORKSPACE",
        "GEMINI_WORKSPACE",
    ] {
        command.env_remove(variable);
    }
    command
}

#[test]
fn help_and_version_exit_without_starting_mcp() {
    let root = tempfile::tempdir().unwrap();
    for option in ["--help", "-h", "--version"] {
        let output = command(root.path()).arg(option).output().unwrap();
        assert!(output.status.success());
        assert!(!output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}

#[test]
fn agent_configuration_and_noninteractive_errors_do_not_start_mcp_or_an_agent() {
    let root = tempfile::tempdir().unwrap();
    for option in ["config", "/config"] {
        let output = command(root.path())
            .env("PATH", "")
            .env("LOCALAPPDATA", root.path())
            .arg(option)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("Antigravity: no instalado"));
        assert!(output.stderr.is_empty());
    }
    let menu = command(root.path()).arg("active").output().unwrap();
    assert!(!menu.status.success());
    assert!(String::from_utf8_lossy(&menu.stderr).contains("necesita una terminal"));
    let unavailable = command(root.path())
        .env("PATH", "")
        .env("LOCALAPPDATA", root.path())
        .args(["active", "antigravity"])
        .output()
        .unwrap();
    assert!(!unavailable.status.success());
    assert!(!root.path().join(".codegraph").exists());
    assert!(!root.path().join(".agents").exists());
}

#[test]
fn index_options_detect_the_project_without_root_argument() {
    let root = tempfile::tempdir().unwrap();
    let implicit = command(root.path()).arg("--sync-plan").output().unwrap();
    let explicit = command(root.path())
        .arg("--root")
        .arg(root.path())
        .arg("--sync-plan")
        .output()
        .unwrap();
    assert_eq!(implicit.status.code(), explicit.status.code());
    assert_eq!(implicit.stdout, explicit.stdout);
    assert_eq!(implicit.stderr, explicit.stderr);
    assert!(!String::from_utf8_lossy(&implicit.stderr).contains("usage:"));
    assert!(!root.path().join(".codegraph").exists());
}

#[test]
fn initialize_index_in_detected_project() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".git")).unwrap();
    let output = command(root.path()).arg("--init-index").output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(root.path().join(".codegraph/codegraph.db").is_file());
}

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
fn index_options_detect_the_project_without_root_argument() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".git")).unwrap();
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
#[test]
fn default_binary_rejects_agent_commands() {
    let root = tempfile::tempdir().unwrap();
    let help = command(root.path()).arg("--help").output().unwrap();
    assert!(!String::from_utf8_lossy(&help.stdout).contains("active antigravity"));
    for args in [
        &["config"][..],
        &["/config"][..],
        &["active", "antigravity"][..],
        &["--root", ".", "active", "antigravity"][..],
    ] {
        let output = command(root.path()).args(args).output().unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("usage:"));
    }
}

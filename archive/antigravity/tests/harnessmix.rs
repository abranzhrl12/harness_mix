use std::{path::Path, process::Command};

#[test]
fn native_hook_denies_without_echoing_the_tool_arguments() {
    use std::io::Write;
    use std::process::Stdio;
    let root = tempfile::tempdir().unwrap();
    let mut child = cli(root.path()).args(["hook", "deny-native"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().unwrap();
    child.stdin.take().unwrap().write_all(br#"{"toolCall":{"name":"view_file","args":{"private":"do-not-echo-this"}}}"#).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["decision"], "deny");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("do-not-echo-this"));
    assert!(output.stderr.is_empty());
}

#[test]
fn hook_profile_is_a_workspace_pretool_gate() {
    let root = tempfile::tempdir().unwrap();
    let output = cli(root.path()).args(["hook", "profile"]).output().unwrap();
    assert!(output.status.success());
    let profile: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let gate = &profile["harnessmix-native-tools"]["PreToolUse"][0];
    let matcher = regex::Regex::new(gate["matcher"].as_str().unwrap()).unwrap();
    assert!(matcher.is_match("view_file"));
    assert!(matcher.is_match("run_command"));
    assert!(!matcher.is_match("harness_harness_read"));
    assert_eq!(gate["hooks"][0]["command"], "harnessmix hook deny-native");
    assert!(!root.path().join(".agents").exists());
}

fn cli(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_harnessmix"));
    command.current_dir(root);
    for variable in [
        "WORKSPACE_ROOT",
        "VSCODE_CWD",
        "INIT_CWD",
        "PROJECT_ROOT",
        "ANTIGRAVITY_WORKSPACE",
        "GEMINI_WORKSPACE",
        "HARNESSMIX_ANTIGRAVITY_ACP",
    ] {
        command.env_remove(variable);
    }
    command
}

#[test]
fn help_and_version_do_not_initialize_a_workspace() {
    let root = tempfile::tempdir().unwrap();
    for argument in ["--help", "--version"] {
        let output = cli(root.path()).arg(argument).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
    assert!(!root.path().join(".codegraph").exists());
}

#[test]
fn config_reports_a_missing_acp_override_without_spawning_it() {
    let root = tempfile::tempdir().unwrap();
    let missing_server = root.path().join("missing-acp-server.exe");
    let output = cli(root.path())
        .env("HARNESSMIX_ANTIGRAVITY_ACP", &missing_server)
        .arg("config")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Antigravity"), "{stdout}");
    assert!(stdout.to_lowercase().contains("no disponible"), "{stdout}");
    assert!(!root.path().join(".codegraph").exists());
}

#[test]
fn config_reports_ask_as_the_default_permission_policy() {
    let root = tempfile::tempdir().unwrap();
    let output = cli(root.path()).args(["config"]).output().unwrap();

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Permisos: preguntar"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn yolo_and_dangerously_skip_permissions_report_auto_approval() {
    let root = tempfile::tempdir().unwrap();
    for flag in ["--yolo", "--dangerously-skip-permissions"] {
        let output = cli(root.path())
            .args(["config", flag])
            .output()
            .unwrap();

        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("Permisos: automáticos"),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn no_animation_is_visible_in_configuration() {
    let root = tempfile::tempdir().unwrap();
    let output = cli(root.path())
        .args(["config", "--no-animation"])
        .output()
        .unwrap();

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Animación: desactivada"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn active_fails_cleanly_for_a_missing_configured_acp_server_without_fallback() {
    let root = tempfile::tempdir().unwrap();
    let missing_server = root.path().join("missing-acp-server.exe");
    let output = cli(root.path())
        .env("HARNESSMIX_ANTIGRAVITY_ACP", &missing_server)
        .args(["active", "--root"])
        .arg(root.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.is_empty(), "expected a useful missing-server error");
    assert!(!root.path().join(".codegraph").exists());
}

#[test]
fn login_reports_a_missing_explicit_agent_command_without_network_access() {
    let root = tempfile::tempdir().unwrap();
    let missing_agent = root.path().join("missing-acp-agent.exe");
    let output = cli(root.path())
        .args(["login", "--root"])
        .arg(root.path())
        .args(["--agent-command"])
        .arg(missing_agent)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let error = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(error.contains("missing-acp-agent.exe"), "{error}");
    assert!(!root.path().join(".codegraph").exists());
}

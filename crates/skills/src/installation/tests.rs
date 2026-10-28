use super::*;

#[test]
fn owned_index_with_idle_writer_gate_can_be_uninstalled() {
    let root = tempfile::tempdir().unwrap();
    let exe = executable(root.path());
    {
        let mut install = Installation::begin(root.path(), &exe).unwrap();
        install.prepare_index().unwrap();
    }
    fs::write(root.path().join(".codegraph/codegraph.db"), b"test index").unwrap();
    fs::write(root.path().join(".codegraph/index-write.lock"), b"").unwrap();
    fs::write(root.path().join(".codegraph/autosync.lock"), b"").unwrap();
    fs::write(root.path().join(".codegraph/autosync.status"), b"harness-watch-status-v1\n{}").unwrap();
    assert_eq!(uninstall(root.path()).unwrap()["complete"], true);
    assert!(!root.path().join(".codegraph").exists());
}

#[test]
fn uninstall_preserves_unrecognized_watcher_status() {
    let root = tempfile::tempdir().unwrap();
    let exe = executable(root.path());
    {
        let mut install = Installation::begin(root.path(), &exe).unwrap();
        install.prepare_index().unwrap();
    }
    fs::write(root.path().join(".codegraph/codegraph.db"), b"index").unwrap();
    let path = root.path().join(".codegraph/autosync.status");
    fs::write(&path, b"foreign").unwrap();
    assert!(uninstall(root.path()).is_err());
    assert_eq!(fs::read(path).unwrap(), b"foreign");
}

#[cfg(windows)]
#[test]
fn active_writer_gate_prevents_uninstall() {
    use std::os::windows::fs::OpenOptionsExt;
    let root = tempfile::tempdir().unwrap();
    let exe = executable(root.path());
    {
        let mut install = Installation::begin(root.path(), &exe).unwrap();
        install.prepare_index().unwrap();
    }
    fs::write(root.path().join(".codegraph/codegraph.db"), b"test index").unwrap();
    let gate = fs::OpenOptions::new().read(true).write(true).create(true)
        .truncate(false).share_mode(3)
        .open(root.path().join(".codegraph/index-write.lock")).unwrap();
    gate.lock().unwrap();
    assert!(uninstall(root.path()).is_err());
    assert!(root.path().join(SKILLS[0].relative_path).exists());
    assert_eq!(fs::read(root.path().join(".codegraph/codegraph.db")).unwrap(), b"test index");
    drop(gate);
    assert_eq!(uninstall(root.path()).unwrap()["complete"], true);
}

fn executable(root: &Path) -> PathBuf {
    let path = root.join("test-harness.exe");
    fs::write(&path, b"test executable placeholder").unwrap();
    path
}

#[test]
fn init_is_complete_idempotent_and_uninstall_removes_only_owned_content() {
    let root = tempfile::tempdir().unwrap();
    let exe = executable(root.path());
    fs::write(root.path().join("user.rs"), b"keep me").unwrap();
    {
        let mut install = Installation::begin(root.path(), &exe).unwrap();
        assert!(install.prepare_index().unwrap());
        fs::write(
            root.path().join(".codegraph/codegraph.db"),
            b"disposable test index",
        )
        .unwrap();
    }
    let before = fs::read(root.path().join(RECEIPT)).unwrap();
    {
        let mut install = Installation::begin(root.path(), &exe).unwrap();
        assert!(!install.prepare_index().unwrap());
    }
    assert_eq!(before, fs::read(root.path().join(RECEIPT)).unwrap());
    for skill in SKILLS {
        assert!(root.path().join(skill.relative_path).is_file());
    }
    assert!(root.path().join(".codex/config.toml").is_file());
    assert!(root.path().join(".agents/mcp_config.json").is_file());
    let report = uninstall(root.path()).unwrap();
    assert_eq!(report["complete"], true, "{report}");
    assert_eq!(fs::read(root.path().join("user.rs")).unwrap(), b"keep me");
    assert!(exe.is_file());
    assert!(!root.path().join(".codegraph").exists());
    assert!(!root.path().join(".agents").exists());
    assert!(!root.path().join(".codex").exists());
    assert_eq!(uninstall(root.path()).unwrap()["complete"], true);
    assert!(!root.path().join(".agents").exists());
}

#[test]
fn preserves_other_servers_settings_preexisting_index_and_preferences() {
    let root = tempfile::tempdir().unwrap();
    let exe = executable(root.path());
    fs::create_dir(root.path().join(".agents")).unwrap();
    fs::create_dir(root.path().join(".codex")).unwrap();
    fs::create_dir(root.path().join(".codegraph")).unwrap();
    fs::write(
        root.path().join(".codegraph/codegraph.db"),
        b"pre-existing shared index",
    )
    .unwrap();
    fs::write(root.path().join(".agents/mcp_config.json"),br#"{"mcpServers":{"other":{"command":"other","env":{"TOKEN":"do-not-copy"}}},"custom":true}"#).unwrap();
    fs::write(
        root.path().join(".codex/config.toml"),
        "# keep this comment\nmodel = 'custom-model'\n[mcp_servers.other]\ncommand = 'other'\n",
    )
    .unwrap();
    fs::write(
        root.path().join(preferences::PATH),
        br#"{"format_version":1,"auto_sync":false}"#,
    )
    .unwrap();
    let original_preferences = fs::read(root.path().join(preferences::PATH)).unwrap();
    {
        let mut install = Installation::begin(root.path(), &exe).unwrap();
        assert!(!install.prepare_index().unwrap());
    }
    let receipt = fs::read_to_string(root.path().join(RECEIPT)).unwrap();
    assert!(!receipt.contains("do-not-copy"));
    // Concurrent user additions to other server sections must survive removal.
    let mut doc = json_document_for_test(root.path());
    doc["later"] = json!(42);
    fs::write(
        root.path().join(".agents/mcp_config.json"),
        serde_json::to_vec(&doc).unwrap(),
    )
    .unwrap();
    let report = uninstall(root.path()).unwrap();
    assert_eq!(report["complete"], true, "{report}");
    let doc = json_document_for_test(root.path());
    assert_eq!(doc["later"], 42);
    assert_eq!(doc["mcpServers"]["other"]["command"], "other");
    assert!(doc["mcpServers"].get("harness").is_none());
    let codex = fs::read_to_string(root.path().join(".codex/config.toml")).unwrap();
    assert!(codex.contains("# keep this comment"));
    assert!(codex.contains("custom-model"));
    assert!(codex.contains("mcp_servers.other"));
    assert!(!codex.contains("mcp_servers.harness"));
    assert_eq!(
        fs::read(root.path().join(preferences::PATH)).unwrap(),
        original_preferences
    );
    assert_eq!(
        fs::read(root.path().join(".codegraph/codegraph.db")).unwrap(),
        b"pre-existing shared index"
    );
}

fn json_document_for_test(root: &Path) -> Value {
    serde_json::from_slice(&fs::read(root.join(".agents/mcp_config.json")).unwrap()).unwrap()
}

#[test]
fn modified_owned_files_are_reported_and_uninstall_can_resume() {
    let root = tempfile::tempdir().unwrap();
    let exe = executable(root.path());
    drop(Installation::begin(root.path(), &exe).unwrap());
    let skill = &SKILLS[0];
    fs::write(root.path().join(skill.relative_path), b"user changed skill").unwrap();
    let report = uninstall(root.path()).unwrap();
    assert_eq!(report["complete"], false);
    assert!(root.path().join(RECEIPT).exists());
    assert_eq!(
        fs::read(root.path().join(skill.relative_path)).unwrap(),
        b"user changed skill"
    );
    fs::write(root.path().join(skill.relative_path), skill.content).unwrap();
    assert_eq!(uninstall(root.path()).unwrap()["complete"], true);
    assert!(!root.path().join(RECEIPT).exists());
}

#[test]
fn client_conflicts_and_malformed_configs_abort_before_installing_skills() {
    for content in [
        "not json",
        r#"{"mcpServers":{"harness":{"command":"user-server"}}}"#,
    ] {
        let root = tempfile::tempdir().unwrap();
        let exe = executable(root.path());
        fs::create_dir(root.path().join(".agents")).unwrap();
        fs::write(root.path().join(".agents/mcp_config.json"), content).unwrap();
        assert!(Installation::begin(root.path(), &exe).is_err());
        assert!(!root.path().join(RECEIPT).exists());
        assert!(!root.path().join(SKILLS[0].relative_path).exists());
        assert_eq!(
            fs::read_to_string(root.path().join(".agents/mcp_config.json")).unwrap(),
            content
        );
    }
}

#[test]
fn forged_receipt_cannot_remove_arbitrary_project_files() {
    let root = tempfile::tempdir().unwrap();
    let exe = executable(root.path());
    drop(Installation::begin(root.path(), &exe).unwrap());
    fs::write(root.path().join("precious.txt"), b"keep me").unwrap();
    let mut receipt: Value =
        serde_json::from_slice(&fs::read(root.path().join(RECEIPT)).unwrap()).unwrap();
    receipt["files"]["precious.txt"] = json!({"before":null,"installed_hash":hash(b"keep me")});
    fs::write(
        root.path().join(RECEIPT),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    assert!(uninstall(root.path()).is_err());
    assert_eq!(
        fs::read(root.path().join("precious.txt")).unwrap(),
        b"keep me"
    );
}

#[test]
fn unknown_index_files_prevent_destructive_uninstall() {
    let root = tempfile::tempdir().unwrap();
    let exe = executable(root.path());
    {
        let mut install = Installation::begin(root.path(), &exe).unwrap();
        install.prepare_index().unwrap();
    }
    fs::write(root.path().join(".codegraph/user-notes.txt"), b"keep me").unwrap();
    assert!(uninstall(root.path()).is_err());
    assert!(root.path().join(SKILLS[0].relative_path).exists());
    assert_eq!(
        fs::read(root.path().join(".codegraph/user-notes.txt")).unwrap(),
        b"keep me"
    );
}

#[test]
fn an_interrupted_file_install_can_be_resumed_without_new_flags() {
    let root = tempfile::tempdir().unwrap();
    let exe = executable(root.path());
    drop(Installation::begin(root.path(), &exe).unwrap());
    fs::remove_file(root.path().join(SKILLS[0].relative_path)).unwrap();
    drop(Installation::begin(root.path(), &exe).unwrap());
    assert_eq!(
        fs::read(root.path().join(SKILLS[0].relative_path)).unwrap(),
        SKILLS[0].content.as_bytes()
    );
    assert_eq!(uninstall(root.path()).unwrap()["complete"], true);
}

#[cfg(windows)]
#[test]
fn linked_agents_directory_is_refused_without_writing_outside_the_project() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let exe = executable(root.path());
    let output=std::process::Command::new("powershell.exe")
        .args(["-NoProfile","-NonInteractive","-Command","New-Item -ItemType Junction -Path $env:HARNESS_INSTALL_TEST_LINK -Target $env:HARNESS_INSTALL_TEST_TARGET | Out-Null"])
        .env("HARNESS_INSTALL_TEST_LINK",root.path().join(".agents"))
        .env("HARNESS_INSTALL_TEST_TARGET",outside.path()).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(Installation::begin(root.path(), &exe).is_err());
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[cfg(windows)]
#[test]
fn live_index_prevents_uninstall_before_removing_project_files() {
    use std::os::windows::fs::OpenOptionsExt;
    let root = tempfile::tempdir().unwrap();
    let exe = executable(root.path());
    {
        let mut install = Installation::begin(root.path(), &exe).unwrap();
        install.prepare_index().unwrap();
    }
    let db = root.path().join(".codegraph/codegraph.db");
    fs::write(&db, b"index").unwrap();
    let _pin = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&db)
        .unwrap();
    assert!(uninstall(root.path()).is_err());
    assert!(root.path().join(SKILLS[0].relative_path).exists());
}

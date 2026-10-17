use agent_harness::adapters::{self, NativeApplication};
use harness_domain::contracts::{GitDiffArgs, GitStatusArgs, Request};
use serde_json::Value;
use std::{path::Path, process::Command};

fn git(root: &Path, args: &[&str]) {
    let mut command = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            command.env_remove(key);
        }
    }
    // Fixture setup only; never mutate the user's repository or run configured hooks/signers.
    let output = command
        .current_dir(root)
        .args([
            "-c",
            "user.name=Harness Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=.no-hooks",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Fixture git command failed: {args:?}"
    );
}
fn status(app: &NativeApplication, offset: usize, limit: usize) -> Value {
    app.execute(Request::GitStatus(GitStatusArgs {
        include_untracked: None,
        offset: Some(offset),
        limit: Some(limit),
        session: None,
    }))
}
fn diff(app: &NativeApplication, path: &str, staged: bool, max_lines: usize) -> Value {
    app.execute(Request::GitDiff(GitDiffArgs {
        path: path.into(),
        staged: Some(staged),
        max_lines: Some(max_lines),
        max_chars: None,
        session: None,
    }))
}
fn fixture() -> tempfile::TempDir {
    let root = tempfile::Builder::new()
        .prefix("harness-git-fixture-")
        .tempdir()
        .unwrap();
    git(root.path(), &["init", "--quiet", "--initial-branch=main"]);
    root
}

#[test]
fn real_status_diff_paging_staging_unicode_and_read_only_index() {
    let root = fixture();
    let app = adapters::application(root.path()).unwrap();
    let initial = status(&app, 0, 30);
    assert_eq!(initial["state"], "ok", "{initial}");
    assert_eq!(initial["data"]["unborn"], true);
    let path = "café file.txt";
    std::fs::write(root.path().join(path), "one\ntwo\n").unwrap();
    git(root.path(), &["add", "--", path]);
    let staged = status(&app, 0, 30);
    assert_eq!(staged["data"]["counts"]["staged"], 1, "{staged}");
    assert_eq!(diff(&app, path, true, 120)["data"]["changed"], true);
    git(
        root.path(),
        &["commit", "--quiet", "--no-verify", "-m", "fixture"],
    );
    std::fs::write(root.path().join(path), "three\ntwo\n").unwrap();
    std::fs::write(root.path().join("untracked.txt"), "new").unwrap();
    let before = std::fs::read(root.path().join(".git/index")).unwrap();
    let page = status(&app, 0, 1);
    assert_eq!(page["data"]["counts"]["modified"], 1, "{page}");
    assert_eq!(page["data"]["counts"]["untracked"], 1);
    assert_eq!(page["data"]["nextOffset"], 1);
    assert!(page["detail"]["ref"].is_string());
    assert_eq!(
        status(&app, 1, 1)["data"]["entries"][0]["path"],
        "untracked.txt"
    );
    let patch = diff(&app, path, false, 1);
    assert_eq!(patch["state"], "ok", "{patch}");
    assert!(patch["detail"]["ref"].is_string(), "{patch}");
    assert_eq!(diff(&app, path, true, 120)["data"]["changed"], false);
    assert_eq!(
        std::fs::read(root.path().join(".git/index")).unwrap(),
        before
    );
    git(root.path(), &["checkout", "--quiet", "--detach"]);
    assert_eq!(status(&app, 0, 30)["data"]["detached"], true);
    std::fs::remove_file(root.path().join(path)).unwrap();
    let deleted = diff(&app, path, false, 120);
    assert_eq!(deleted["state"], "ok", "{deleted}");
    assert!(
        deleted["data"]["preview"]
            .as_str()
            .unwrap()
            .contains("deleted file mode")
    );
}

#[test]
fn unsafe_filters_and_fsmonitor_are_disabled_without_execution() {
    let root = fixture();
    std::fs::write(root.path().join("sample.txt"), "before\n").unwrap();
    git(root.path(), &["add", "sample.txt"]);
    git(
        root.path(),
        &["commit", "--quiet", "--no-verify", "-m", "fixture"],
    );
    std::fs::write(
        root.path().join(".gitattributes"),
        "*.txt filter=unsafe diff=unsafe\n",
    )
    .unwrap();
    std::fs::write(root.path().join(".filter-fixture-allowed"), "fixture only").unwrap();
    let exe = std::env::current_exe()
        .unwrap()
        .to_str()
        .unwrap()
        .replace('\\', "/");
    let command = format!("\"{exe}\" --exact injected_filter_fixture --nocapture");
    for key in [
        "filter.unsafe.clean",
        "filter.unsafe.process",
        "diff.external",
        "diff.unsafe.textconv",
        "core.fsmonitor",
    ] {
        git(root.path(), &["config", key, &command]);
    }
    git(root.path(), &["config", "filter.unsafe.required", "true"]);
    std::fs::write(root.path().join("sample.txt"), "after!\n").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = status(&app, 0, 30);
    assert_eq!(result["state"], "ok", "{result}");
    let patch = diff(&app, "sample.txt", false, 120);
    assert_eq!(patch["state"], "ok", "{patch}");
    assert!(
        patch["data"]["preview"]
            .as_str()
            .unwrap()
            .contains("after!")
    );
    assert!(!root.path().join("FILTER_EXECUTED").exists());
}

#[test]
fn injected_filter_fixture() {
    // Only the malicious test configuration above can opt this child into writing.
    if Path::new(".filter-fixture-allowed").is_file() {
        std::fs::write("FILTER_EXECUTED", "unsafe filter was executed").unwrap();
        panic!("Git must never launch this fixture");
    }
}

#[test]
fn rejects_secret_paths_metadata_and_non_repository_scopes() {
    let root = fixture();
    let app = adapters::application(root.path()).unwrap();
    for path in [
        ".env",
        ".git/config",
        "web.config",
        "appsettings.json",
        ".npmrc",
        "../outside",
        "missing/../../outside",
        "missing/.env",
        "missing/NUL",
        "missing/x:stream",
    ] {
        assert_eq!(
            diff(&app, path, false, 120)["diagnostics"][0]["code"],
            "PATH"
        );
    }
    assert_eq!(status(&app, 0, 0)["diagnostics"][0]["code"], "ARGUMENT");
    let nonrepo = tempfile::tempdir().unwrap();
    let nonrepo = adapters::application(nonrepo.path()).unwrap();
    assert_eq!(
        status(&nonrepo, 0, 30)["diagnostics"][0]["code"],
        "GIT_REPO"
    );
}

#[test]
fn binary_output_bounds_and_external_worktree_config_fail_closed() {
    let root = fixture();
    std::fs::write(root.path().join("binary.dat"), b"\0one").unwrap();
    std::fs::write(root.path().join("large.txt"), "short\n").unwrap();
    git(root.path(), &["add", "binary.dat", "large.txt"]);
    git(
        root.path(),
        &["commit", "--quiet", "--no-verify", "-m", "fixture"],
    );
    let app = adapters::application(root.path()).unwrap();
    std::fs::write(root.path().join("binary.dat"), b"\0two").unwrap();
    assert_eq!(diff(&app, "binary.dat", false, 120)["data"]["binary"], true);
    std::fs::write(
        root.path().join("large.txt"),
        "x".repeat(2 * 1024 * 1024 + 1),
    )
    .unwrap();
    let bounded = diff(&app, "large.txt", false, 120);
    assert_eq!(bounded["diagnostics"][0]["code"], "GIT_SIZE", "{bounded}");
    let outside = tempfile::tempdir().unwrap();
    git(
        root.path(),
        &["config", "core.worktree", outside.path().to_str().unwrap()],
    );
    assert_eq!(status(&app, 0, 30)["diagnostics"][0]["code"], "GIT_REPO");
}

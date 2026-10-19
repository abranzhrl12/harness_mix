use agent_harness::adapters::{self, NativeApplication};
use harness_domain::contracts::{GitArgs, GitDiffArgs, GitStatusArgs};
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
    app.execute(
        GitArgs::Status(GitStatusArgs {
            include_untracked: None,
            offset: Some(offset),
            limit: Some(limit),
            session: None,
        })
        .into_request(),
    )
}
fn diff(app: &NativeApplication, path: &str, staged: bool, max_lines: usize) -> Value {
    app.execute(
        GitArgs::Diff(GitDiffArgs {
            path: path.into(),
            staged: Some(staged),
            max_lines: Some(max_lines),
            max_chars: None,
            session: None,
        })
        .into_request(),
    )
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
fn directory_and_workspace_diff_filter_credentials_and_preserve_staging() {
    let root = fixture();
    std::fs::create_dir(root.path().join("src")).unwrap();
    for (path, text) in [
        ("src/a.txt", "old-a\n"),
        ("src/b.txt", "old-b\n"),
        ("other.txt", "old-other\n"),
        ("src/Web.config", "private-before\n"),
    ] {
        std::fs::write(root.path().join(path), text).unwrap();
    }
    git(root.path(), &["add", "."]);
    git(
        root.path(),
        &["commit", "--quiet", "--no-verify", "-m", "fixture"],
    );
    for (path, text) in [
        ("src/a.txt", "new-a\n"),
        ("src/b.txt", "new-b\n"),
        ("other.txt", "new-other\n"),
        ("src/Web.config", "private-after\n"),
    ] {
        std::fs::write(root.path().join(path), text).unwrap();
    }
    let app = adapters::application(root.path()).unwrap();
    let directory = diff(&app, "src", false, 500);
    assert_eq!(directory["state"], "ok", "{directory}");
    let preview = directory["data"]["preview"].as_str().unwrap();
    assert!(preview.contains("new-a") && preview.contains("new-b"));
    assert!(!preview.contains("new-other") && !preview.contains("private"));
    assert_eq!(directory["data"]["files"]["excludedByPolicy"], 1);
    let workspace = diff(&app, ".", false, 500);
    assert!(
        workspace["data"]["preview"]
            .as_str()
            .unwrap()
            .contains("new-other"),
        "{workspace}"
    );
    assert!(!workspace.to_string().contains("private-after"));
    git(root.path(), &["add", "src/a.txt"]);
    let staged = diff(&app, ".", true, 500);
    assert!(
        staged["data"]["preview"]
            .as_str()
            .unwrap()
            .contains("new-a")
    );
    assert!(
        !staged["data"]["preview"]
            .as_str()
            .unwrap()
            .contains("new-b")
    );
    std::fs::remove_file(root.path().join("src/b.txt")).unwrap();
    assert!(
        diff(&app, "src", false, 500)["data"]["preview"]
            .as_str()
            .unwrap()
            .contains("deleted file")
    );
}

fn linked_fixture() -> (tempfile::TempDir, tempfile::TempDir, std::path::PathBuf) {
    let main = fixture();
    std::fs::write(main.path().join("tracked.txt"), "before\n").unwrap();
    git(main.path(), &["add", "tracked.txt"]);
    git(
        main.path(),
        &["commit", "--quiet", "--no-verify", "-m", "fixture"],
    );
    let linked = tempfile::Builder::new()
        .prefix("harness linked café ")
        .tempdir()
        .unwrap();
    git(
        main.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "feature",
            linked.path().to_str().unwrap(),
        ],
    );
    let pointer = std::fs::read_to_string(linked.path().join(".git")).unwrap();
    let directory = Path::new(pointer.strip_prefix("gitdir: ").unwrap().trim_end())
        .canonicalize()
        .unwrap();
    (main, linked, directory)
}

#[test]
fn linked_worktree_status_and_diff_preserve_both_indexes_and_disable_filters() {
    let (main, linked, directory) = linked_fixture();
    let app = adapters::application(linked.path()).unwrap();
    assert_eq!(status(&app, 0, 30)["data"]["branch"], "feature");
    std::fs::write(linked.path().join("tracked.txt"), "staged\n").unwrap();
    git(linked.path(), &["add", "tracked.txt"]);
    std::fs::write(linked.path().join("tracked.txt"), "worktree\n").unwrap();
    std::fs::write(
        linked.path().join(".gitattributes"),
        "*.txt filter=unsafe diff=unsafe\n",
    )
    .unwrap();
    std::fs::write(
        linked.path().join(".filter-fixture-allowed"),
        "fixture only",
    )
    .unwrap();
    let exe = std::env::current_exe()
        .unwrap()
        .to_str()
        .unwrap()
        .replace('\\', "/");
    let command = format!("\"{exe}\" --exact injected_filter_fixture --nocapture");
    // Include per-worktree configuration as well as the common repository config.
    git(
        main.path(),
        &["config", "extensions.worktreeConfig", "true"],
    );
    for key in [
        "filter.unsafe.clean",
        "filter.unsafe.process",
        "diff.external",
        "diff.unsafe.textconv",
        "core.fsmonitor",
    ] {
        git(linked.path(), &["config", "--worktree", key, &command]);
    }
    git(
        linked.path(),
        &["config", "--worktree", "filter.unsafe.required", "true"],
    );
    let main_index = std::fs::read(main.path().join(".git/index")).unwrap();
    let linked_index = std::fs::read(directory.join("index")).unwrap();
    let result = status(&app, 0, 30);
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["counts"]["staged"], 1);
    assert_eq!(result["data"]["counts"]["modified"], 1);
    for (staged, expected) in [(true, "+staged"), (false, "+worktree")] {
        let patch = diff(&app, "tracked.txt", staged, 120);
        assert_eq!(patch["state"], "ok", "{patch}");
        assert!(
            patch["data"]["preview"]
                .as_str()
                .unwrap()
                .contains(expected),
            "{patch}"
        );
    }
    assert!(!linked.path().join("FILTER_EXECUTED").exists());
    assert!(!main.path().join("FILTER_EXECUTED").exists());
    assert_eq!(
        std::fs::read(main.path().join(".git/index")).unwrap(),
        main_index
    );
    assert_eq!(
        std::fs::read(directory.join("index")).unwrap(),
        linked_index
    );
}

#[test]
fn linked_worktree_relative_gitfile_and_invalid_registration() {
    let (main, linked, directory) = linked_fixture();
    // Both isolated fixtures share the temp parent; preserve the supported relative layout.
    let relative = format!(
        "../{}/.git/worktrees/{}",
        main.path().file_name().unwrap().to_str().unwrap(),
        directory.file_name().unwrap().to_str().unwrap()
    );
    std::fs::write(linked.path().join(".git"), format!("gitdir: {relative}\n")).unwrap();
    let app = adapters::application(linked.path()).unwrap();
    let result = status(&app, 0, 30);
    assert_eq!(result["state"], "ok", "{result}");
    // A gitfile cannot borrow another repository's metadata without a reciprocal registration.
    std::fs::write(
        directory.join("gitdir"),
        main.path().join(".git").to_str().unwrap(),
    )
    .unwrap();
    assert_eq!(status(&app, 0, 30)["diagnostics"][0]["code"], "GIT_REPO");
    assert_eq!(
        diff(&app, "tracked.txt", false, 120)["diagnostics"][0]["code"],
        "GIT_REPO"
    );
    std::fs::write(
        directory.join("gitdir"),
        linked.path().join(".git").to_str().unwrap(),
    )
    .unwrap();
    std::fs::write(directory.join("commondir"), main.path().to_str().unwrap()).unwrap();
    assert_eq!(status(&app, 0, 30)["diagnostics"][0]["code"], "GIT_REPO");
}

#[test]
fn linked_worktree_metadata_aliases_and_worktree_redirection_are_rejected() {
    let (main, linked, directory) = linked_fixture();
    let app = adapters::application(linked.path()).unwrap();
    git(
        main.path(),
        &["config", "extensions.worktreeConfig", "true"],
    );
    git(
        linked.path(),
        &[
            "config",
            "--worktree",
            "core.worktree",
            main.path().to_str().unwrap(),
        ],
    );
    let redirected = status(&app, 0, 30);
    assert_eq!(
        redirected["diagnostics"][0]["code"], "GIT_REPO",
        "{redirected}"
    );
    git(
        linked.path(),
        &["config", "--worktree", "--unset", "core.worktree"],
    );
    let alias = linked.path().join("metadata-alias");
    #[cfg(windows)]
    {
        let output = Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&alias)
            .arg(&directory)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&directory, &alias).unwrap();
    #[cfg(any(windows, unix))]
    {
        std::fs::write(
            linked.path().join(".git"),
            format!("gitdir: {}\n", alias.display()),
        )
        .unwrap();
        let result = status(&app, 0, 30);
        assert_eq!(result["diagnostics"][0]["code"], "GIT_REPO", "{result}");
        #[cfg(windows)]
        std::fs::remove_dir(&alias).unwrap();
        #[cfg(unix)]
        std::fs::remove_file(&alias).unwrap();
    }
}

#[test]
fn malformed_and_unregistered_gitfiles_fail_with_sanitized_errors() {
    let main = fixture();
    let workspace = tempfile::tempdir().unwrap();
    let app = adapters::application(workspace.path()).unwrap();
    for text in [
        "not a gitfile".to_string(),
        "gitdir: private\nsecond-line".to_string(),
        format!("gitdir: {}\n", main.path().join(".git").display()),
        format!("gitdir: {}", "x".repeat(8192)),
    ] {
        std::fs::write(workspace.path().join(".git"), text).unwrap();
        let result = status(&app, 0, 30);
        assert_eq!(result["diagnostics"][0]["code"], "GIT_REPO", "{result}");
        assert!(!result.to_string().contains("private"));
    }
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

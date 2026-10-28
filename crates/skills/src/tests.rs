use super::*;
use crate::lock::SyncLock;
use std::{fs, io, path::Path};
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

#[test]
fn fresh_sync_and_repeat_are_idempotent() {
    let temp = tempdir().unwrap();
    let first = sync(temp.path(), false, false).unwrap();
    assert_eq!(first.conflicts, 0);
    assert_eq!(first.errors, 0);
    assert_eq!(first.changes, SKILLS.len() + 1);
    let second = sync(temp.path(), false, false).unwrap();
    assert_eq!(second.changes, 0);
    assert_eq!(second.conflicts, 0);
}

#[test]
fn updates_only_a_file_matching_the_prior_manifest_hash() {
    let temp = tempdir().unwrap();
    sync(temp.path(), false, false).unwrap();
    let skill = &SKILLS[0];
    let file = temp.path().join(skill.relative_path);
    fs::write(&file, "old managed content").unwrap();
    let manifest_path = temp.path().join(MANIFEST_PATH);
    let mut manifest: Manifest =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest.files.insert(
        skill.relative_path.into(),
        harness_domain::hash(b"old managed content"),
    );
    fs::write(manifest_path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    let report = sync(temp.path(), false, false).unwrap();
    assert_eq!(report.conflicts, 0);
    assert_eq!(fs::read(file).unwrap(), skill.content.as_bytes());
}

#[test]
fn preserves_unmanaged_and_previously_managed_local_edits() {
    let temp = tempdir().unwrap();
    let skill = &SKILLS[0];
    let file = temp.path().join(skill.relative_path);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "local edit").unwrap();
    let report = sync(temp.path(), false, false).unwrap();
    assert_eq!(report.conflicts, 1);
    assert_eq!(fs::read(&file).unwrap(), b"local edit");

    let temp = tempdir().unwrap();
    sync(temp.path(), false, false).unwrap();
    let file = temp.path().join(skill.relative_path);
    fs::write(&file, "local edit after install").unwrap();
    let report = sync(temp.path(), false, false).unwrap();
    assert_eq!(report.conflicts, 1);
    assert_eq!(fs::read(file).unwrap(), b"local edit after install");
}

#[test]
fn check_and_dry_run_do_not_create_directories() {
    let temp = tempdir().unwrap();
    let report = sync(temp.path(), true, false).unwrap();
    assert_eq!(report.changes, SKILLS.len() + 1);
    assert!(!temp.path().join(".agents").exists());
    sync(temp.path(), false, true).unwrap();
    assert!(!temp.path().join(".agents").exists());
}

#[test]
fn rejects_parent_traversal() {
    assert!(validate_relative(Path::new("../outside/SKILL.md")).is_err());
    assert!(validate_relative(Path::new("/outside/SKILL.md")).is_err());
}

#[test]
fn malformed_manifest_fails_before_any_write() {
    let temp = tempdir().unwrap();
    let manifest = temp.path().join(MANIFEST_PATH);
    fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    fs::write(&manifest, "not json").unwrap();
    assert!(sync(temp.path(), false, false).is_err());
    assert_eq!(fs::read(&manifest).unwrap(), b"not json");
    assert!(!temp.path().join(".agents/skills/harness-read").exists());
}

#[test]
fn rejects_symlinked_skill_parent_and_workspace_root() {
    let temp = tempdir().unwrap();
    let outside = temp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    let skills_dir = temp.path().join(".agents/skills");
    fs::create_dir_all(skills_dir.parent().unwrap()).unwrap();
    create_directory_symlink(&outside, &skills_dir).unwrap();
    assert!(sync(temp.path(), false, false).is_err());
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);

    let temp = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let root_link = temp.path().join("linked-workspace");
    create_directory_symlink(outside.path(), &root_link).unwrap();
    assert!(sync(&root_link, false, false).is_err());
    assert!(!outside.path().join(".agents").exists());
}

#[test]
fn auto_sync_is_opt_in_and_disableable() {
    let temp = tempdir().unwrap();
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
    let temp = tempdir().unwrap();
    run(
        temp.path(),
        Options {
            enable_auto: true,
            ..Options::default()
        },
    )
    .unwrap();
    let skill = &SKILLS[0];
    let file = temp.path().join(skill.relative_path);
    fs::write(&file, "keep this local edit").unwrap();
    let report = sync_on_startup(temp.path()).unwrap().unwrap();
    assert_eq!(report.conflicts, 1);
    assert_eq!(report.errors, 0);
    assert_eq!(fs::read(file).unwrap(), b"keep this local edit");
}

#[test]
fn lock_is_exclusive_until_dropped() {
    let temp = tempdir().unwrap();
    let lock = SyncLock::acquire(temp.path()).unwrap();
    assert!(SyncLock::acquire(temp.path()).is_err());
    drop(lock);
    assert!(SyncLock::acquire(temp.path()).is_ok());
}

#[test]
fn harness_skills_are_in_english_and_document_new_capabilities() {
    let find_skill = |id: &str| {
        SKILLS
            .iter()
            .find(|s| s.id == id && s.relative_path.ends_with("SKILL.md"))
            .unwrap_or_else(|| panic!("missing skill {id}"))
            .content
    };

    let read = find_skill("harness-read");
    assert!(read.contains("description: Investigate code, read symbols, and query dependencies with Harness MCP."));
    assert!(read.contains("inline per-job summaries"));
    assert!(read.contains("detail.ref"));
    assert!(read.contains("form field → JSON/JS → C# model → SP parameter → SQL column"));

    let edit = find_skill("harness-edit");
    assert!(edit.contains("description: Edit code and verify changes with Harness MCP."));
    assert!(edit.contains("fast per-file verification"));
    assert!(edit.contains("validated"));
    assert!(edit.contains("pending"));

    let browser = find_skill("harness-browser");
    assert!(browser.contains("description: Diagnose and verify web applications with Harness MCP"));
    assert!(browser.contains("selectedTab"));
    assert!(browser.contains("availableTabs"));
    assert!(browser.contains("\"auto\""));
    assert!(browser.contains("\"localhost\""));

    let sql = find_skill("harness-sql");
    assert!(sql.contains("description: Query SQL Server or PostgreSQL databases via Harness MCP"));

    let xlsx = find_skill("harness-xlsx");
    assert!(xlsx.contains("description: Inspect or read existing XLSX workbooks via Harness MCP"));

    let svg = find_skill("harness-svg");
    assert!(svg.contains("description: Inspect or preview existing static SVG files with Harness MCP."));

    let setup = find_skill("harness-setup");
    assert!(setup.contains("description: Install or uninstall Harness in a project, connect MCP, and prepare initial usage."));
}

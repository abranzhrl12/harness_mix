use super::*;
use crate::adapters::index_profile as profile;
use std::os::windows::fs::OpenOptionsExt;

struct ProfileGuard;
impl Drop for ProfileGuard {
    fn drop(&mut self) { profile::finish(); }
}

fn same_result(project: &Project, path: &str, limit: u64) {
    let context = OperationContext::default();
    let before = project.index_source_bytes_checked(path, &context, limit);
    let after = project.index_source_bytes_pinned(path, &context, limit);
    match (before, after) {
        (Ok(before), Ok(after)) => assert_eq!(before, after, "{path}"),
        (Err(before), Err(after)) => assert_eq!(before.0, after.0, "{path}"),
        (before, after) => panic!("{path}: checked={before:?}, pinned={after:?}"),
    }
}

#[test]
fn pinned_read_matches_checked_bytes_limits_and_absence() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("nested/área")).unwrap();
    fs::write(root.path().join("nested/área/file.rs"), b"abc").unwrap();
    fs::write(root.path().join("empty.rs"), []).unwrap();
    fs::write(root.path().join("binary.rs"), [0xff, 0, 0xfe]).unwrap();
    let project = Project::new(root.path()).unwrap();
    for path in ["nested/área/file.rs", "nested\\área\\file.rs",
        "./nested//área/file.rs", "empty.rs", "binary.rs", "nested/absent.rs",
        "missing/ancestor/file.rs", "nested", "."] {
        for limit in [0, 2, 3, 1024, u64::MAX] { same_result(&project, path, limit); }
    }
}

#[test]
fn pinned_read_preserves_shared_lexical_restrictions() {
    let root = tempfile::tempdir().unwrap();
    let project = Project::new(root.path()).unwrap();
    for path in ["", "../outside", "nested/../file.rs", "C:\\outside", "/outside",
        "\\\\server\\share", "a\0b", ".env", ".env.local", "nested/secret.txt",
        "credentials.json", "mcp_config.json", "oauth_creds.json", "id_rsa",
        "NUL.rs", "CON", "COM1.txt", "file.", "file ", "a?b"] {
        same_result(&project, path, 1024);
        assert_eq!(project.index_source_bytes_pinned(path, &OperationContext::default(), 1024)
            .unwrap_err().0, "PATH", "{path}");
    }
}

#[test]
fn pinned_read_does_not_reuse_content_or_keep_handles_after_return() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("nested")).unwrap();
    let path = root.path().join("nested/file.rs");
    fs::write(&path, b"alpha").unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let project = Project::new(root.path()).unwrap();
    let context = OperationContext::default();
    assert_eq!(project.index_source_bytes_pinned("nested/file.rs", &context, 1024).unwrap(), Some(b"alpha".to_vec()));
    fs::write(&path, b"bravo").unwrap();
    fs::OpenOptions::new().write(true).open(&path).unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified)).unwrap();
    assert_eq!(project.index_source_bytes_pinned("nested/file.rs", &context, 1024).unwrap(), Some(b"bravo".to_vec()));
    fs::rename(root.path().join("nested"), root.path().join("renamed")).unwrap();
    same_result(&project, "nested/file.rs", 1024);
    same_result(&project, "renamed/file.rs", 1024);
}

#[test]
fn scoped_reads_keep_canonical_identity_checks() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("allowed.rs"), b"allowed").unwrap();
    fs::write(root.path().join("other.rs"), b"other").unwrap();
    fs::write(root.path().join("appsettings.json"), b"{}").unwrap();
    let project = Project::new(root.path()).unwrap();
    let single = Project { single_file: Some(root.path().join("allowed.rs").canonicalize().unwrap()), ..project.clone() };
    let external = Project { external: true, ..project };
    profile::begin(true);
    let _profile = ProfileGuard;
    for scope in [&single, &external] {
        for path in ["allowed.rs", "other.rs", "appsettings.json", "missing.rs"] {
            same_result(scope, path, 1024);
        }
    }
    assert!(profile::metrics().unwrap()["operations"].get("source_path_policy").is_none());
    assert_eq!(single.index_source_bytes_pinned("other.rs", &OperationContext::default(), 1024).unwrap_err().0, "PATH");
    assert_eq!(external.index_source_bytes_pinned("appsettings.json", &OperationContext::default(), 1024).unwrap_err().0, "PATH");
}

#[test]
fn pinned_read_checks_cancellation_and_deadline_before_io() {
    let root = tempfile::tempdir().unwrap();
    let project = Project::new(root.path()).unwrap();
    let cancelled = OperationContext::default();
    cancelled.cancel();
    let expired = OperationContext::with_timeout(std::time::Duration::ZERO);
    for (context, code) in [(&cancelled, "CANCELLED"), (&expired, "TIMEOUT")] {
        assert_eq!(project.index_source_bytes_pinned("missing/file.rs", context, 1024).unwrap_err().0, code);
    }
}

#[test]
fn opened_handle_must_belong_to_the_project_before_reading_bytes() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(root.path().join("inside.rs"), b"inside").unwrap();
    fs::write(outside.path().join("outside.rs"), b"outside").unwrap();
    let project = Project::new(root.path()).unwrap();
    project.verify_opened_source(&File::open(root.path().join("inside.rs")).unwrap()).unwrap();
    assert_eq!(project.verify_opened_source(&File::open(outside.path().join("outside.rs")).unwrap()).unwrap_err().0, "PATH");
    profile::begin(true);
    let _profile = ProfileGuard;
    assert!(project.read_input_path_with(outside.path().join("outside.rs"), &OperationContext::default(), 1024,
        |file| project.verify_opened_source(file)).is_err());
    assert!(profile::metrics().unwrap()["operations"].get("io_read_bytes").is_none());
}

#[test]
fn junction_inserted_after_lexical_build_is_rejected_before_leaf_read() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("file.rs"), b"outside").unwrap();
    let project = Project::new(root.path()).unwrap();
    project.build_target("junction/file.rs", false, |_| Ok(())).unwrap();
    let junction = root.path().join("junction");
    let output = std::process::Command::new("cmd").args(["/c", "mklink", "/J"])
        .arg(&junction).arg(outside.path()).output().unwrap();
    assert!(output.status.success());
    profile::begin(true);
    let _profile = ProfileGuard;
    for path in ["junction/file.rs", "junction/missing.rs", "junction/missing/deep.rs"] {
        same_result(&project, path, 1024);
        assert_eq!(project.index_source_bytes_pinned(path, &OperationContext::default(), 1024).unwrap_err().0, "PATH");
    }
    assert!(profile::metrics().unwrap()["operations"].get("io_open").is_none());
    // Remove the junction entry, never recursively traverse its target.
    fs::remove_dir(junction).unwrap();
}

#[test]
fn pin_failure_is_not_reported_as_absence() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("nested");
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("file.rs"), b"present").unwrap();
    let project = Project::new(root.path()).unwrap();
    let _blocker = File::options().read(true).custom_flags(0x0200_0000)
        .share_mode(0).open(&parent).unwrap();
    profile::begin(true);
    let _profile = ProfileGuard;
    same_result(&project, "nested/file.rs", 1024);
    assert!(project.index_source_bytes_pinned("nested/file.rs", &OperationContext::default(), 1024).is_err());
    assert!(profile::metrics().unwrap()["counters"]["source_pin_fallback"].as_u64().unwrap() >= 1);
}

#[test]
fn pinned_path_checks_once_and_directory_handles_prevent_rename() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("nested");
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("file.rs"), b"bytes").unwrap();
    let project = Project::new(root.path()).unwrap();
    profile::begin(true);
    let _profile = ProfileGuard;
    project.index_source_bytes_pinned("nested/file.rs", &OperationContext::default(), 1024).unwrap();
    let metrics = profile::metrics().unwrap();
    assert!(metrics["operations"].get("source_path_guard").is_none());
    for name in ["source_path_policy", "source_pin_directory", "source_metadata", "io_open", "io_metadata_before", "io_metadata_after"] {
        assert_eq!(metrics["operations"][name]["calls"], 1, "{name}");
    }
    let pins = super::super::edit_commit::pin_directory(&project, &project.root.join("nested")).unwrap();
    assert!(fs::rename(&parent, root.path().join("renamed")).is_err());
    drop(pins);
    fs::rename(&parent, root.path().join("renamed")).unwrap();
}

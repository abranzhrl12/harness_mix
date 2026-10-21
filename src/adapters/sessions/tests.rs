use super::*;
use crate::adapters::project::Project;
use serde_json::json;
use std::fs;

#[test]
fn active_session_is_not_evicted_and_capacity_recovers_after_release() {
    let dir = tempfile::tempdir().unwrap();
    let mut sessions = Sessions::new(Project::new(dir.path()).unwrap());
    sessions.limit = 1;
    let (first, _) = sessions.create().unwrap();
    sessions.acquire(&first).unwrap();
    assert_eq!(sessions.create().unwrap_err().0, "SESSION_LIMIT");
    assert!(!sessions.get(&first).unwrap().restored);
    sessions.release(&first);
    sessions.create().unwrap();
    assert!(sessions.get(&first).unwrap().restored);
}

#[test]
fn restores_node_v1_and_drops_freshness_and_untrusted_fields() {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::new(dir.path()).unwrap();
    let mut first = Sessions::new(project.clone());
    let (id, warning) = first.create().unwrap();
    assert!(warning.is_none());
    let mut data = Session::default().state;
    data.declared.goal = "Continuar migración".into();
    data.checks = vec![
        json!({"language":"rust","scope":"core","time":"now","ok":true,"current":true,"version":"untrusted"}),
    ];
    first.save(&id, &data).unwrap();
    let mut second = Sessions::new(project);
    let loaded = second.get(&id).unwrap();
    assert!(loaded.restored);
    assert_eq!(loaded.state.declared.goal, "Continuar migración");
    assert!(loaded.state.checks[0].get("current").is_none());
    assert!(loaded.state.checks[0].get("version").is_none());
    assert!(loaded.rules_hash.is_none());
    assert!(second.get("../outside").is_err());
}

#[test]
fn memory_eviction_can_restore_from_disk_and_reject_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let mut sessions = Sessions::new(Project::new(dir.path()).unwrap());
    sessions.limit = 1;
    let (first, _) = sessions.create().unwrap();
    sessions.create().unwrap();
    assert!(sessions.get(&first).unwrap().restored);
    sessions.entries.clear();
    fs::write(
        dir.path()
            .join(format!(".agents/harness/.state/{first}.json")),
        b"{}",
    )
    .unwrap();
    assert!(sessions.get(&first).is_err());
}

#[cfg(windows)]
#[test]
fn persistence_does_not_write_through_junctions() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".agents/harness")).unwrap();
    let junction = dir.path().join(".agents").join("harness").join(".state");
    let result = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&junction)
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let mut sessions = Sessions::new(Project::new(dir.path()).unwrap());
    let (_, warning) = sessions.create().unwrap();
    assert_eq!(warning.unwrap()["code"], "PERSISTENCE");
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    fs::remove_dir(junction).unwrap();
}

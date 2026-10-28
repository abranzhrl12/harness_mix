use super::*;
use crate::adapters::index_store;
use harness_domain::execution::OperationContext;
use std::fs;

#[test]
fn advisory_record_round_trips_and_recovers_interrupted_payload() {
    let root = tempfile::tempdir().unwrap();
    index_store::initialize(root.path(), &OperationContext::default()).unwrap();
    let status = json!({"ownerPid":123,"ownerStartedAtUnixSeconds":100,"state":"idle",
        "lastSuccessfulSyncUnixSeconds":105,"lastError":null});
    publish(root.path(), &status, 110).unwrap();
    let observed = observe(root.path()).unwrap();
    assert_eq!(observed["pid"], 123);
    assert_eq!(observed["heartbeatUnixSeconds"], 110);
    assert_eq!(observed["lastSuccessfulSyncUnixSeconds"], 105);
    fs::write(root.path().join(".codegraph/autosync.status"), b"harness-watch-status-v1\n{").unwrap();
    assert!(observe(root.path()).is_none());
    publish(root.path(), &status, 120).unwrap();
    assert_eq!(observe(root.path()).unwrap()["heartbeatUnixSeconds"], 120);
}

#[test]
fn unknown_or_hardlinked_status_is_never_overwritten() {
    let root = tempfile::tempdir().unwrap();
    index_store::initialize(root.path(), &OperationContext::default()).unwrap();
    let path = root.path().join(".codegraph/autosync.status");
    fs::write(&path, b"foreign").unwrap();
    assert!(publish(root.path(), &json!({}), 1).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"foreign");
    fs::remove_file(&path).unwrap();
    let other = root.path().join("other.txt");
    fs::write(&other, b"").unwrap();
    fs::hard_link(&other, &path).unwrap();
    assert!(publish(root.path(), &json!({}), 1).is_err());
    assert!(fs::read(other).unwrap().is_empty());
}

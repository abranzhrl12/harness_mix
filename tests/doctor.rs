use agent_harness::adapters;
use harness_domain::contracts::{DoctorArgs, Request};
use serde_json::Value;

fn doctor(app: &adapters::NativeApplication, versions: bool) -> Value {
    app.execute(Request::Doctor(DoctorArgs {
        versions: Some(versions),
        connection: Some(true),
        session: None,
    }))
}
#[test]
fn diagnosis_does_not_create_state_or_claim_remote_connections() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = doctor(&app, false);
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["coverage"], "native_local_only");
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    let checks = result["data"]["checks"].as_array().unwrap();
    for id in ["connection:codegraph", "sql"] {
        assert_eq!(
            checks.iter().find(|c| c["id"] == id).unwrap()["status"],
            "skipped"
        );
    }
    for id in ["tool:git", "tool:cargo", "tool:rustc"] {
        assert_eq!(
            checks.iter().find(|c| c["id"] == id).unwrap()["versionChecked"],
            false
        );
    }
    std::fs::write(root.path().join(".codegraph"), "not a directory").unwrap();
    let invalid = doctor(&app, false);
    assert_eq!(invalid["data"]["health"], "error");
    std::fs::remove_file(root.path().join(".codegraph")).unwrap();
    std::fs::create_dir(root.path().join(".codegraph")).unwrap();
    let unverified = doctor(&app, false);
    let check = unverified["data"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "index")
        .unwrap();
    assert_eq!(check["status"], "warning");
}

#[test]
fn requested_version_probes_only_report_sanitized_numeric_versions() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = doctor(&app, true);
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["versionsRequested"], true);
    let checks = result["data"]["checks"].as_array().unwrap();
    let mut completed = 0;
    for check in checks
        .iter()
        .filter(|c| c["id"].as_str().unwrap().starts_with("tool:"))
    {
        if check["versionChecked"] == true {
            completed += 1;
            assert!(
                check["numericVersion"]
                    .as_str()
                    .unwrap()
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '.')
            );
        }
        assert!(check.get("stdout").is_none() && check.get("stderr").is_none());
    }
    assert!(
        completed > 0,
        "Fixture requires at least one of Git/Cargo/Rustc installed: {result}"
    );
}

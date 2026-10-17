use agent_harness::adapters;
use harness_domain::contracts::{Request, VerifyAction, VerifyArgs};
use serde_json::Value;

fn verify(application: &adapters::NativeApplication, action: VerifyAction) -> Value {
    application.execute(Request::Verify(VerifyArgs {
        language: "rust".into(),
        scope: Some("crate with spaces".into()),
        action: Some(action),
        session: None,
    }))
}

#[test]
fn real_cargo_build_tests_and_failed_build_have_correct_envelopes() {
    let root = tempfile::tempdir().unwrap();
    let crate_root = root.path().join("crate with spaces");
    std::fs::create_dir_all(crate_root.join("src")).unwrap();
    std::fs::write(crate_root.join("Cargo.toml"), "[package]\nname='native-verification-fixture'\nversion='0.1.0'\nedition='2024'\n[workspace]\n").unwrap();
    std::fs::write(
        crate_root.join("src/lib.rs"),
        "#[test] fn works() { assert_eq!(2+2,4); }\n",
    )
    .unwrap();
    let application = adapters::application(root.path()).unwrap();
    for action in [VerifyAction::Build, VerifyAction::Test] {
        let is_test = matches!(action, VerifyAction::Test);
        let result = verify(&application, action);
        assert_eq!(result["state"], "ok", "{result}");
        assert_eq!(result["data"]["passed"], true, "{result}");
        assert_eq!(result["data"]["verificationFreshness"], "not_tracked");
        // The first build may create Cargo.lock; subsequent tests must be stable.
        if is_test {
            assert_eq!(
                result["data"]["workspaceObservation"]["status"], "unchanged",
                "{result}"
            );
        }
    }
    std::fs::write(crate_root.join("build.rs"), r#"fn main() {
        let root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        std::fs::write(std::path::Path::new(&root).join("generated.txt"), "created during build").unwrap();
    }"#).unwrap();
    let result = verify(&application, VerifyAction::Build);
    assert_eq!(result["data"]["passed"], true, "{result}");
    assert_eq!(
        result["data"]["workspaceObservation"]["status"], "changed",
        "{result}"
    );
    std::fs::write(root.path().join(".env"), "restricted fixture").unwrap();
    let result = verify(&application, VerifyAction::Build);
    assert_eq!(result["data"]["passed"], true, "{result}");
    assert_eq!(
        result["data"]["workspaceObservation"]["status"], "unknown",
        "{result}"
    );
    assert_eq!(
        result["data"]["workspaceObservation"]["beforeError"],
        "PATH"
    );
    std::fs::remove_file(root.path().join(".env")).unwrap();
    std::fs::write(
        crate_root.join("src/lib.rs"),
        "compile_error!(\"intentional fixture error\");\n",
    )
    .unwrap();
    let result = verify(&application, VerifyAction::Build);
    assert_eq!(result["state"], "error", "{result}");
    assert_eq!(result["data"]["passed"], false);
    assert_eq!(result["diagnostics"][0]["code"], "VERIFY_FAILED");
}

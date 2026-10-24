#[test]
fn manifest_scope_cannot_escape_or_become_an_option() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("crate with spaces")).unwrap();
    std::fs::write(
        root.path().join("crate with spaces/Cargo.toml"),
        "[package]\n",
    )
    .unwrap();
    let project = Project::new(root.path()).unwrap();
    let mut args = VerifyArgs {
        project: None,
        configuration: None,
        engine: None,
        warnings: None,
        language: "rust".into(),
        package: None,
        scope: Some("crate with spaces".into()),
        action: Some(VerifyAction::Test),
        session: None,
        timeout_seconds: None,
    };
    let spec = plan(&project, &args).unwrap();
    assert_eq!(spec.arguments()[0], "test");
    assert_eq!(spec.arguments()[1], "--manifest-path");
    assert!(spec.arguments()[2].ends_with("Cargo.toml"));
    for scope in [
        "../Cargo.toml",
        "C:/Cargo.toml",
        "--help",
        "crate with spaces/Cargo.toml/..",
    ] {
        args.scope = Some(scope.into());
        assert!(plan(&project, &args).is_err(), "{scope}");
    }
    args.language = "unknown-language".into();
    assert_eq!(plan(&project, &args).unwrap_err().0, "UNSUPPORTED");
}

#[test]
fn package_plan_defaults_to_check_with_fifteen_minute_process_budget() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("Cargo.toml"), "[workspace]\n").unwrap();
    let project = Project::new(root.path()).unwrap();
    let args: VerifyArgs =
        serde_json::from_value(json!({"language":"rust","package":"domain"})).unwrap();
    let spec = verification_plan(&project, &args).unwrap();
    assert_eq!(spec.arguments()[0], "check");
    assert_eq!(&spec.arguments()[5..], &["-p", "domain"]);
    assert_eq!(spec.timeout(), args.effective_timeout().unwrap());
    assert_eq!(
        harness_domain::contracts::Request::Verify(args).execution_timeout(),
        Duration::from_secs(905)
    );
}

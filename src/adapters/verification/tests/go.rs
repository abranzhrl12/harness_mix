#[test]
fn go_plan_has_closed_arguments_and_accepts_module_with_spaces() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("module with spaces")).unwrap();
    std::fs::write(
        root.path().join("module with spaces/go.mod"),
        "module example.test/fixture\ngo 1.20\n",
    )
    .unwrap();
    let project = Project::new(root.path()).unwrap();
    for action in [VerifyAction::Build, VerifyAction::Test] {
        let test = matches!(action, VerifyAction::Test);
        let args = VerifyArgs {
            project: None,
            configuration: None,
            engine: None,
            warnings: None,
            timeout_seconds: None,
            language: "GO".into(),
            package: None,
            scope: Some("module with spaces/go.mod".into()),
            action: Some(action),
            session: None,
        };
        let spec = plan(&project, &args).unwrap();
        assert_eq!(spec.program(), Program::Go);
        assert_eq!(spec.arguments()[0], "-C");
        assert!(spec.arguments()[1].ends_with("module with spaces"));
        assert_eq!(spec.arguments()[2], if test { "test" } else { "build" });
        assert_eq!(
            spec.arguments().last().unwrap(),
            if test { "./..." } else { "." }
        );
        assert!(spec.arguments().contains(&"-mod=readonly".into()));
        assert!(spec.arguments().contains(&"-buildvcs=false".into()));
        assert_eq!(spec.arguments().contains(&"-count=1".into()), test);
        assert_eq!(spec.timeout(), Duration::from_secs(30));
    }
}

#[test]
fn go_scope_rejects_files_traversal_and_nonmodules_before_execution() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("main.go"), "package main").unwrap();
    std::fs::create_dir(root.path().join("empty")).unwrap();
    let project = Project::new(root.path()).unwrap();
    for scope in [
        "main.go",
        "empty",
        "../go.mod",
        "C:/go.mod",
        "--help",
        ".\\..\\go.mod",
    ] {
        let args = VerifyArgs {
            project: None,
            configuration: None,
            engine: None,
            warnings: None,
            timeout_seconds: None,
            language: "go".into(),
            package: None,
            scope: Some(scope.into()),
            action: None,
            session: None,
        };
        assert!(plan(&project, &args).is_err(), "{scope}");
    }
}

#[test]
#[ignore = "requires an installed Go toolchain; run explicitly"]
fn real_go_build_test_and_failure() {
    super::super::process::executable(Program::Go)
        .expect("Go executable unavailable in absolute PATH");
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("go.mod"),
        "module example.test/nativefixture\ngo 1.20\n",
    )
    .unwrap();
    std::fs::write(
        root.path().join("fixture.go"),
        "package fixture\nfunc Add(a, b int) int { return a+b }\n",
    )
    .unwrap();
    std::fs::write(root.path().join("fixture_test.go"), "package fixture\nimport \"testing\"\nfunc TestAdd(t *testing.T) { if Add(2,2)!=4 { t.Fatal(\"unexpected result\") } }\n").unwrap();
    let app = super::super::application(root.path()).unwrap();
    for action in [VerifyAction::Build, VerifyAction::Test] {
        let result = app.execute(harness_domain::contracts::Request::Verify(VerifyArgs {
            project: None,
            configuration: None,
            engine: None,
            warnings: None,
            timeout_seconds: None,
            language: "go".into(),
            package: None,
            scope: None,
            action: Some(action),
            session: None,
        }));
        assert_eq!(result["state"], "ok", "{result}");
        assert_eq!(result["scope"]["language"], "go", "{result}");
        assert_eq!(result["data"]["passed"], true, "{result}");
        assert_eq!(result["data"]["verificationFreshness"], "not_tracked");
    }
    std::fs::write(
        root.path().join("fixture.go"),
        "package fixture\ninvalid syntax\n",
    )
    .unwrap();
    let result = app.execute(harness_domain::contracts::Request::Verify(VerifyArgs {
        project: None,
        configuration: None,
        engine: None,
        warnings: None,
        timeout_seconds: None,
        language: "go".into(),
        package: None,
        scope: None,
        action: None,
        session: None,
    }));
    assert_eq!(result["state"], "error", "{result}");
    assert_eq!(result["data"]["passed"], false);
    assert_eq!(result["diagnostics"][0]["code"], "VERIFY_FAILED");
}

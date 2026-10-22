#[test]
fn python_plans_use_isolated_stdlib_commands_and_validated_scopes() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file with spaces.py"), "x=1\n").unwrap();
    std::fs::create_dir(root.path().join("tests with spaces")).unwrap();
    let project = Project::new(root.path()).unwrap();
    let mut args = VerifyArgs {
        project: None,
        configuration: None,
        engine: None,
        warnings: None,
        language: "PYTHON".into(),
        package: None,
        scope: Some("file with spaces.py".into()),
        action: None,
        session: None,
    };
    let spec = plan(&project, &args).unwrap();
    assert_eq!(spec.program(), Program::Python);
    assert_eq!(&spec.arguments()[..4], ["-I", "-S", "-m", "py_compile"]);
    assert!(spec.arguments()[4].ends_with("file with spaces.py"));
    args.scope = Some("tests with spaces".into());
    args.action = Some(VerifyAction::Test);
    let spec = plan(&project, &args).unwrap();
    assert_eq!(
        &spec.arguments()[..6],
        ["-I", "-S", "-B", "-m", "unittest", "discover"]
    );
    assert_eq!(spec.arguments()[6], "-s");
    assert_eq!(spec.arguments()[8], "-t");
    assert_eq!(spec.arguments()[7], spec.arguments()[9]);
    assert_eq!(&spec.arguments()[10..], ["-p", "test*.py"]);
}

#[test]
fn python_rejects_implicit_scope_wrong_types_and_traversal() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.py"), "pass").unwrap();
    std::fs::write(root.path().join("a.txt"), "pass").unwrap();
    let project = Project::new(root.path()).unwrap();
    for scope in [
        None,
        Some("."),
        Some("a.txt"),
        Some("../a.py"),
        Some("--help"),
        Some("C:/a.py"),
    ] {
        let args = VerifyArgs {
            project: None,
            configuration: None,
            engine: None,
            warnings: None,
            language: "python".into(),
            package: None,
            scope: scope.map(String::from),
            action: None,
            session: None,
        };
        assert!(plan(&project, &args).is_err());
    }
    let args = VerifyArgs {
        project: None,
        configuration: None,
        engine: None,
        warnings: None,
        language: "python".into(),
        package: None,
        scope: Some("a.txt".into()),
        action: Some(VerifyAction::Test),
        session: None,
    };
    assert_eq!(plan(&project, &args).unwrap_err().0, "VERIFY_SCOPE");
}

#[test]
#[ignore = "requires installed Python 3; run explicitly"]
fn real_python_compilation_unittest_and_failures() {
    super::super::process::executable(Program::Python)
        .expect("Python unavailable in absolute PATH");
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("file with spaces.py"),
        "raise RuntimeError('compile must not execute source')\n",
    )
    .unwrap();
    std::fs::write(
        root.path().join("py_compile.py"),
        "raise RuntimeError('local py_compile must not load')\n",
    )
    .unwrap();
    std::fs::write(
        root.path().join("sitecustomize.py"),
        "open('startup-marker.txt','w').write('bad')\n",
    )
    .unwrap();
    std::fs::create_dir(root.path().join("tests with spaces")).unwrap();
    std::fs::write(root.path().join("tests with spaces/test_fixture.py"), "import unittest\nclass Fixture(unittest.TestCase):\n def test_ok(self): self.assertEqual(2+2,4)\n").unwrap();
    let app = super::super::application(root.path()).unwrap();
    let call = |scope: &str, action| {
        app.execute(harness_domain::contracts::Request::Verify(VerifyArgs {
            project: None,
            configuration: None,
            engine: None,
            warnings: None,
            language: "python".into(),
            package: None,
            scope: Some(scope.into()),
            action: Some(action),
            session: None,
        }))
    };
    let build = call("file with spaces.py", VerifyAction::Build);
    assert_eq!(build["state"], "ok", "{build}");
    assert_eq!(build["scope"]["language"], "python");
    assert_eq!(build["data"]["verificationMode"], "py_compile_syntax_only");
    assert!(!root.path().join("startup-marker.txt").exists());
    let test = call("tests with spaces", VerifyAction::Test);
    assert_eq!(test["state"], "ok", "{test}");
    assert!(
        test["data"]["output"]
            .as_str()
            .unwrap()
            .contains("Ran 1 test")
    );
    assert_eq!(test["data"]["testCountVerified"], false);
    std::fs::write(root.path().join("tests with spaces/test_fixture.py"), "import unittest\nclass Fixture(unittest.TestCase):\n def test_bad(self): self.fail('expected fixture failure')\n").unwrap();
    let failed = call("tests with spaces", VerifyAction::Test);
    assert_eq!(failed["state"], "error", "{failed}");
    assert_eq!(failed["diagnostics"][0]["code"], "VERIFY_FAILED");
    std::fs::write(root.path().join("file with spaces.py"), "def broken(:\n").unwrap();
    assert_eq!(
        call("file with spaces.py", VerifyAction::Build)["state"],
        "error"
    );
}

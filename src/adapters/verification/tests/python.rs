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
        timeout_seconds: None,
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
            timeout_seconds: None,
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
        timeout_seconds: None,
        language: "python".into(),
        package: None,
        scope: Some("a.txt".into()),
        action: Some(VerifyAction::Test),
        session: None,
    };
    assert_eq!(plan(&project, &args).unwrap_err().0, "VERIFY_SCOPE");
}

#[test]
fn python_fast_per_file_verification_and_symbols_and_edit_validation() {
    let root = tempfile::tempdir().unwrap();
    let py_source = "\
class Service(BaseService):
    \"\"\"Main service.\"\"\"
    @classmethod
    def create(cls, name: str):
        return cls(name)

    async def run(self, count: int) -> int:
        def helper(v: int) -> int:
            return v + 1
        return helper(count)

def top_level(flag: bool = True) -> bool:
    return flag
";
    std::fs::write(root.path().join("service.py"), py_source).unwrap();
    let app = super::super::application(root.path()).unwrap();

    // 1. harness_symbols outline on .py
    let outline_res = app.execute(harness_domain::contracts::Request::Outline(
        harness_domain::contracts::OutlineArgs {
            path: "service.py".into(),
            external_scope: None,
            max_chars: None,
            query: None,
            view: None,
            filter_kind: None,
            session: None,
        },
    ));
    assert_eq!(outline_res["state"], "ok", "{outline_res}");
    assert_eq!(outline_res["data"]["language"], "python");
    let items = outline_res["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 5, "{items:?}");
    assert_eq!(items[0]["name"], "Service");
    assert_eq!(items[0]["kind"], "class");
    assert_eq!(items[0]["container"], "");
    assert_eq!(items[0]["signature"], "class Service(BaseService):");
    assert_eq!(items[1]["name"], "create");
    assert_eq!(items[1]["kind"], "method");
    assert_eq!(items[1]["container"], "Service");
    assert!(items[1]["signature"].as_str().unwrap().contains("def create(cls, name: str):"));
    assert_eq!(items[2]["name"], "run");
    assert_eq!(items[2]["kind"], "method");
    assert_eq!(items[2]["container"], "Service");
    assert_eq!(items[2]["signature"], "async def run(self, count: int) -> int:");
    assert_eq!(items[3]["name"], "helper");
    assert_eq!(items[3]["kind"], "function");
    assert_eq!(items[3]["container"], "Service::run");
    assert_eq!(items[3]["signature"], "def helper(v: int) -> int:");
    assert_eq!(items[4]["name"], "top_level");
    assert_eq!(items[4]["kind"], "function");
    assert_eq!(items[4]["container"], "");
    assert_eq!(items[4]["signature"], "def top_level(flag: bool = True) -> bool:");

    // 2. harness_symbols read on .py
    let read_res = app.execute(harness_domain::contracts::Request::ReadSymbol(
        harness_domain::contracts::ReadSymbolArgs {
            path: "service.py".into(),
            external_scope: None,
            symbol: "run".into(),
            container: Some("Service".into()),
            symbol_id: None,
            view: None,
            session: None,
        },
    ));
    assert_eq!(read_res["state"], "ok", "{read_res}");
    assert_eq!(read_res["data"]["symbol"]["kind"], "method");
    assert!(read_res["data"]["text"].as_str().unwrap().contains("async def run"));

    // 3. harness_verify with engine="fast" on .py
    let verify_ok = app.execute(harness_domain::contracts::Request::Verify(VerifyArgs {
        project: None,
        configuration: None,
        engine: Some("fast".into()),
        warnings: None,
        timeout_seconds: None,
        language: "python".into(),
        package: None,
        scope: Some("service.py".into()),
        action: None,
        session: None,
    }));
    assert_eq!(verify_ok["state"], "ok", "{verify_ok}");
    assert_eq!(verify_ok["data"]["fastPerFile"], true);
    assert_eq!(verify_ok["data"]["verificationMode"], "python_fast_per_file");

    // 4. harness_replace on .py without allowUnvalidatedText: rejects invalid syntax, accepts valid syntax
    let bad_replace = app.execute(harness_domain::contracts::Request::Replace(
        harness_domain::contracts::ReplaceArgs {
            bom: None,
            path: "service.py".into(),
            hash: String::new(),
            line: None,
            chunks: None,
            old_text: "return flag".into(),
            new_text: "return (flag".into(),
            allow_unvalidated_text: false,
            preview: false,
            session: None,
        },
    ));
    assert_eq!(bad_replace["state"], "error", "{bad_replace}");
    assert_eq!(bad_replace["diagnostics"][0]["code"], "SYNTAX_ERROR");

    let good_replace = app.execute(harness_domain::contracts::Request::Replace(
        harness_domain::contracts::ReplaceArgs {
            bom: None,
            path: "service.py".into(),
            hash: String::new(),
            line: None,
            chunks: None,
            old_text: "return flag".into(),
            new_text: "return not flag".into(),
            allow_unvalidated_text: false,
            preview: false,
            session: None,
        },
    ));
    assert_eq!(good_replace["state"], "ok", "{good_replace}");
    assert_eq!(good_replace["data"]["syntax"]["language"], "python");
    assert_eq!(good_replace["data"]["syntax"]["status"], "valid");
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
            timeout_seconds: None,
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

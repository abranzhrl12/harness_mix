#[test]
fn javascript_plan_uses_closed_commands_for_explicit_files() {
    let root = tempfile::tempdir().unwrap();
    let project = Project::new(root.path()).unwrap();
    for extension in ["js", "cjs", "mjs"] {
        let scope = format!("file with spaces.{extension}");
        std::fs::write(
            root.path().join(&scope),
            "throw new Error('not executed by check');",
        )
        .unwrap();
        for action in [VerifyAction::Build, VerifyAction::Test] {
            let test = matches!(action, VerifyAction::Test);
            let spec = plan(
                &project,
                &VerifyArgs {
                    project: None,
                    configuration: None,
                    engine: None,
                    warnings: None,
                    timeout_seconds: None,
                    language: "JS".into(),
                    package: None,
                    scope: Some(scope.clone()),
                    action: Some(action),
                    session: None,
                },
            )
            .unwrap();
            assert_eq!(spec.program(), Program::Node);
            assert_eq!(spec.arguments()[0], if test { "--test" } else { "--check" });
            assert!(spec.arguments().last().unwrap().ends_with(&scope));
            assert_eq!(
                spec.arguments().contains(&"--test-concurrency=1".into()),
                test
            );
            assert_eq!(spec.arguments().len(), if test { 4 } else { 2 });
        }
    }
}

#[test]
fn javascript_rejects_implicit_scope_and_path_injection() {
    let root = tempfile::tempdir().unwrap();
    for file in ["a.ts", "a.jsx", "a.json", "pattern[1].js"] {
        std::fs::write(root.path().join(file), "{}").unwrap();
    }
    let project = Project::new(root.path()).unwrap();
    for scope in [
        None,
        Some("."),
        Some("a.jsx"),
        Some("a.json"),
        Some("pattern[1].js"),
        Some("../a.js"),
        Some("--eval"),
        Some("C:/a.js"),
    ] {
        let args = VerifyArgs {
            project: None,
            configuration: None,
            engine: None,
            warnings: None,
            timeout_seconds: None,
            language: "javascript".into(),
            package: None,
            scope: scope.map(String::from),
            action: None,
            session: None,
        };
        assert!(plan(&project, &args).is_err());
    }
    for language in ["javascript", "typescript", "ts"] {
        let args = VerifyArgs {
            project: None,
            configuration: None,
            engine: None,
            warnings: None,
            timeout_seconds: None,
            language: language.into(),
            package: None,
            scope: Some("a.ts".into()),
            action: None,
            session: None,
        };
        assert!(plan(&project, &args).is_err());
        assert!(matches!(
            background_plan(root.path(), &args, None).unwrap(),
            BackgroundVerificationPlan::TypeScriptSyntax
        ));
    }
}

#[test]
fn typescript_and_tsx_builds_validate_syntax_without_execution_or_typechecking() {
    let root = tempfile::tempdir().unwrap();
    let project = Project::new(root.path()).unwrap();
    for (scope, source) in [
        (
            "valid.ts",
            "const value: number = 'not type checked'; throw new Error('must not execute');",
        ),
        (
            "component.tsx",
            "const value: number = 1; export const element = <section>{value}</section>;",
        ),
    ] {
        std::fs::write(root.path().join(scope), source).unwrap();
        let args = VerifyArgs {
            project: None,
            configuration: None,
            engine: None,
            warnings: None,
            timeout_seconds: None,
            language: "typescript".into(),
            package: None,
            scope: Some(scope.into()),
            action: Some(VerifyAction::Build),
            session: None,
        };
        let result = verify(&project, &args, &OperationContext::default()).unwrap();
        assert_eq!(result["passed"], true, "{result}");
        assert_eq!(result["language"], "typescript");
        assert_eq!(result["verificationMode"], "typescript_syntax_only");
        assert_eq!(result["typeCheckPerformed"], false);
        assert_eq!(result["codeExecuted"], false);
        assert!(matches!(
            background_plan(root.path(), &args, None).unwrap(),
            BackgroundVerificationPlan::TypeScriptSyntax
        ));
    }
}

#[test]
fn typescript_reloads_changed_source_and_rejects_invalid_syntax_and_tests() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("check.ts");
    std::fs::write(&path, "const value: number = 1;").unwrap();
    let mut args = VerifyArgs {
        project: None,
        configuration: None,
        engine: None,
        warnings: None,
        timeout_seconds: None,
        language: "ts".into(),
        package: None,
        scope: Some("check.ts".into()),
        action: Some(VerifyAction::Build),
        session: None,
    };
    background_plan(root.path(), &args, None).unwrap();
    std::fs::write(&path, "const value: = 1;").unwrap();
    let outcome = run_typescript_syntax(root.path(), &args, &OperationContext::default()).unwrap();
    assert_eq!(outcome.exit_code, Some(1));
    assert_eq!(outcome.reason, StopReason::Exited);
    let project = Project::new(root.path()).unwrap();
    let result = verify(&project, &args, &OperationContext::default()).unwrap();
    assert_eq!(result["passed"], false);
    args.action = Some(VerifyAction::Test);
    assert!(matches!(
        background_plan(root.path(), &args, None),
        Err(("UNSUPPORTED", _))
    ));
    assert!(matches!(
        verify(&project, &args, &OperationContext::default()),
        Err(("UNSUPPORTED", _))
    ));
    args.action = Some(VerifyAction::Build);
    let context = OperationContext::without_deadline();
    context.cancel();
    assert!(matches!(
        run_typescript_syntax(root.path(), &args, &context),
        Err(("CANCELLED", _))
    ));
    std::fs::write(path, " ".repeat(512 * 1024 + 1)).unwrap();
    assert!(run_typescript_syntax(root.path(), &args, &OperationContext::default()).is_err());
}

#[test]
#[ignore = "requires installed Node.js with builtin test runner; run explicitly"]
fn real_javascript_check_tests_and_failures() {
    super::super::process::executable(Program::Node).expect("Node unavailable in absolute PATH");
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("check with spaces.cjs"),
        "throw new Error('syntax check must not execute');\n",
    )
    .unwrap();
    std::fs::write(root.path().join("example.mjs"), "export const value = 4;\n").unwrap();
    std::fs::write(root.path().join("fixture.test.cjs"), "const test=require('node:test'); const assert=require('node:assert/strict'); test('fixture',()=>assert.equal(2+2,4));\n").unwrap();
    let app = super::super::application(root.path()).unwrap();
    let call = |scope: &str, action| {
        app.execute(harness_domain::contracts::Request::Verify(VerifyArgs {
            project: None,
            configuration: None,
            engine: None,
            warnings: None,
            timeout_seconds: None,
            language: "javascript".into(),
            package: None,
            scope: Some(scope.into()),
            action: Some(action),
            session: None,
        }))
    };
    for scope in ["check with spaces.cjs", "example.mjs"] {
        let result = call(scope, VerifyAction::Build);
        assert_eq!(result["state"], "ok", "{result}");
        assert_eq!(result["scope"]["language"], "javascript");
        assert_eq!(result["data"]["verificationMode"], "node_syntax_check");
    }
    let test = call("fixture.test.cjs", VerifyAction::Test);
    assert_eq!(test["state"], "ok", "{test}");
    assert!(
        test["data"]["output"]
            .as_str()
            .unwrap()
            .contains("# pass 1")
    );
    assert_eq!(test["data"]["testCountVerified"], false);
    std::fs::write(
        root.path().join("fixture.test.cjs"),
        "const test=require('node:test'); test('bad',()=>{throw new Error('expected failure')});\n",
    )
    .unwrap();
    let failed = call("fixture.test.cjs", VerifyAction::Test);
    assert_eq!(failed["state"], "error", "{failed}");
    assert_eq!(failed["diagnostics"][0]["code"], "VERIFY_FAILED");
    std::fs::write(
        root.path().join("check with spaces.cjs"),
        "function broken( {",
    )
    .unwrap();
    assert_eq!(
        call("check with spaces.cjs", VerifyAction::Build)["state"],
        "error"
    );
}

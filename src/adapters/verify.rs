use super::{process::NativeProcessRunner, project::Project};
use harness_application::ports::ProcessRunner;
use harness_domain::{
    Failure,
    contracts::{VerifyAction, VerifyArgs},
    execution::OperationContext,
    process::{ProcessSpec, Program, StopReason},
    results::{prefix, utf16_len},
};
use serde_json::{Value, json};
use std::time::Duration;

fn plan(project: &Project, args: &VerifyArgs) -> Result<ProcessSpec, Failure> {
    if args.language.eq_ignore_ascii_case("dotnet") || args.language.eq_ignore_ascii_case("csharp")
    {
        return dotnet_plan(project, args);
    }
    if args.language.eq_ignore_ascii_case("javascript")
        || args.language.eq_ignore_ascii_case("js")
        || args.language.eq_ignore_ascii_case("typescript")
        || args.language.eq_ignore_ascii_case("ts")
    {
        return javascript_plan(project, args);
    }
    if args.language.eq_ignore_ascii_case("python") {
        return python_plan(project, args);
    }
    if args.language.eq_ignore_ascii_case("go") {
        return go_plan(project, args);
    }
    if !args.language.eq_ignore_ascii_case("rust") {
        return Err((
            "UNSUPPORTED",
            "Only Rust, Go, Python, JavaScript and dotnet verification are migrated",
        ));
    }
    let scope = args.scope.as_deref().unwrap_or(".");
    if scope.len() > 4096 {
        return Err(("PATH", "Scope too long"));
    }
    let target = project.target(scope)?;
    let manifest = if target.is_dir() {
        project.target(&format!(
            "{}/Cargo.toml",
            scope.trim_end_matches(['/', '\\'])
        ))?
    } else {
        if target.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml") {
            return Err((
                "VERIFY_SCOPE",
                "Rust scope must be a crate directory or Cargo.toml",
            ));
        }
        target
    };
    if !manifest.is_file() {
        return Err(("VERIFY_SCOPE", "Cargo.toml is not a file"));
    }
    let action = match args.action {
        Some(VerifyAction::Test) => "test",
        _ => "build",
    };
    ProcessSpec::new(
        Program::Cargo,
        vec![
            action.into(),
            "--manifest-path".into(),
            manifest
                .to_str()
                .ok_or(("PATH", "Manifest path must be UTF-8"))?
                .into(),
            "--color".into(),
            "never".into(),
        ],
        Duration::from_secs(30),
        2 * 1024 * 1024,
    )
}

fn dotnet_plan(project: &Project, args: &VerifyArgs) -> Result<ProcessSpec, Failure> {
    let scope = args
        .scope
        .as_deref()
        .ok_or(("VERIFY_SCOPE", ".NET requires an explicit .csproj or .sln"))?;
    if scope.len() > 4096 {
        return Err(("PATH", "Scope too long"));
    }
    let target = project.target(scope)?;
    if !target.is_file()
        || !target
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                ext.eq_ignore_ascii_case("csproj") || ext.eq_ignore_ascii_case("sln")
            })
    {
        return Err((
            "VERIFY_SCOPE",
            ".NET scope must be a .csproj or .sln file, not a directory",
        ));
    }
    let relative = target
        .strip_prefix(&project.root)
        .ok()
        .and_then(|p| p.to_str())
        .ok_or(("PATH", ".NET scope must be relative UTF-8"))?;
    #[cfg(windows)]
    let relative = relative.replace('\\', "/");
    // No auto-discovery of neighboring projects, free MSBuild flags, restore or servers.
    ProcessSpec::new(
        Program::Dotnet,
        vec![
            if matches!(args.action, Some(VerifyAction::Test)) {
                "test".into()
            } else {
                "build".into()
            },
            format!("./{relative}"),
            "--no-restore".into(),
            "--nologo".into(),
            "--verbosity".into(),
            "minimal".into(),
            "--disable-build-servers".into(),
            "-nodeReuse:false".into(),
            "-maxCpuCount:1".into(),
            "-p:UseSharedCompilation=false".into(),
        ],
        Duration::from_secs(30),
        2 * 1024 * 1024,
    )
}

fn javascript_plan(project: &Project, args: &VerifyArgs) -> Result<ProcessSpec, Failure> {
    let scope = args
        .scope
        .as_deref()
        .ok_or(("VERIFY_SCOPE", "JavaScript requires an explicit file"))?;
    if scope.len() > 4096 {
        return Err(("PATH", "Scope too long"));
    }
    let target = project.target(scope)?;
    let is_ts = target
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ["ts", "tsx"].contains(&ext));

    if !target.is_file()
        || !target
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ["js", "cjs", "mjs", "ts", "tsx"].contains(&ext))
    {
        return Err((
            "VERIFY_SCOPE",
            "JavaScript/TypeScript scope must be a .js/.cjs/.mjs/.ts/.tsx file",
        ));
    }
    let mut arguments = if matches!(args.action, Some(VerifyAction::Test)) {
        vec![
            "--test".into(),
            "--test-reporter=tap".into(),
            "--test-concurrency=1".into(),
        ]
    } else if is_ts {
        vec![
            "--no-warnings".into(),
            "--check".into(),
        ]
    } else {
        vec!["--check".into()]
    };
    let relative = target
        .strip_prefix(&project.root)
        .ok()
        .and_then(|path| path.to_str())
        .ok_or(("PATH", "JavaScript scope must be relative UTF-8"))?;
    if relative.contains(['*', '?', '[', ']', '{', '}', '(', ')', '!']) {
        return Err((
            "VERIFY_SCOPE",
            "JavaScript scope cannot contain glob metacharacters",
        ));
    }
    #[cfg(windows)]
    let relative = relative.replace('\\', "/");
    // Node's Windows test globber does not reliably accept verbatim \\?\ paths.
    // Prefix prevents option injection; the path was resolved and validated above.
    arguments.push(format!("./{relative}"));
    ProcessSpec::new(
        Program::Node,
        arguments,
        Duration::from_secs(30),
        2 * 1024 * 1024,
    )
}

fn python_plan(project: &Project, args: &VerifyArgs) -> Result<ProcessSpec, Failure> {
    let scope = args.scope.as_deref().ok_or((
        "VERIFY_SCOPE",
        "Python requires explicit file (build) or test directory (test)",
    ))?;
    if scope.len() > 4096 {
        return Err(("PATH", "Scope too long"));
    }
    let target = project.target(scope)?;
    let path = target
        .to_str()
        .ok_or(("PATH", "Python scope must be UTF-8"))?;
    let arguments = if matches!(args.action, Some(VerifyAction::Test)) {
        if !target.is_dir() {
            return Err((
                "VERIFY_SCOPE",
                "Python test requires a directory for test discovery",
            ));
        }
        let has_pytest = target.join("pytest.ini").is_file()
            || target.join("pyproject.toml").is_file()
            || target.join("conftest.py").is_file()
            || project.root.join("pytest.ini").is_file()
            || project.root.join("pyproject.toml").is_file();

        if has_pytest {
            vec![
                "-B".into(),
                "-m".into(),
                "pytest".into(),
                path.into(),
                "-q".into(),
            ]
        } else {
            vec![
                "-I".into(),
                "-S".into(),
                "-B".into(),
                "-m".into(),
                "unittest".into(),
                "discover".into(),
                "-s".into(),
                path.into(),
                "-t".into(),
                path.into(),
                "-p".into(),
                "test*.py".into(),
            ]
        }
    } else {
        if !target.is_file() || target.extension().and_then(|e| e.to_str()) != Some("py") {
            return Err(("VERIFY_SCOPE", "Python build requires a .py file"));
        }
        vec![
            "-I".into(),
            "-S".into(),
            "-m".into(),
            "py_compile".into(),
            path.into(),
        ]
    };
    ProcessSpec::new(
        Program::Python,
        arguments,
        Duration::from_secs(30),
        2 * 1024 * 1024,
    )
}

fn go_plan(project: &Project, args: &VerifyArgs) -> Result<ProcessSpec, Failure> {
    let scope = args.scope.as_deref().unwrap_or(".");
    if scope.len() > 4096 {
        return Err(("PATH", "Scope too long"));
    }
    let target = project.target(scope)?;
    let (directory, manifest) = if target.is_dir() {
        (
            target,
            project.target(&format!("{}/go.mod", scope.trim_end_matches(['/', '\\'])))?,
        )
    } else if target.file_name().and_then(|n| n.to_str()) == Some("go.mod") {
        (
            target
                .parent()
                .ok_or(("VERIFY_SCOPE", "Go module directory unavailable"))?
                .to_path_buf(),
            target,
        )
    } else {
        return Err((
            "VERIFY_SCOPE",
            "Go scope must be a module directory or go.mod",
        ));
    };
    if !manifest.is_file() {
        return Err(("VERIFY_SCOPE", "go.mod is not a file"));
    }
    let test = matches!(args.action, Some(VerifyAction::Test));
    let mut arguments = vec![
        "-C".into(),
        directory
            .to_str()
            .ok_or(("PATH", "Go scope must be UTF-8"))?
            .into(),
        if test { "test".into() } else { "build".into() },
        "-mod=readonly".into(),
        "-buildvcs=false".into(),
    ];
    if test {
        arguments.extend(["-count=1".into(), "-timeout=20s".into()]);
        arguments.push("./...".into());
    } else {
        arguments.push(".".into());
    }
    ProcessSpec::new(
        Program::Go,
        arguments,
        Duration::from_secs(30),
        2 * 1024 * 1024,
    )
}

pub(super) fn verify(
    project: &Project,
    args: &VerifyArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    let spec = plan(project, args)?;
    let runner = NativeProcessRunner::new(&project.root)?;
    let before = super::fingerprint::capture(project, context);
    context.check()?;
    let outcome = runner.run(&spec, context)?;
    // A failed observation must not hide the tool's actual result or pretend freshness.
    let after = super::fingerprint::capture(project, context);
    let mut observation = match (&before, &after) {
        (Ok(a), Ok(b)) => {
            json!({"status":if a == b { "unchanged" } else { "changed" }, "before":a,"after":b})
        }
        _ => {
            json!({"status":"unknown","beforeError":before.as_ref().err().map(|e| e.0),"afterError":after.as_ref().err().map(|e| e.0)})
        }
    };
    observation["coverage"] = super::fingerprint::coverage();
    let stdout = String::from_utf8_lossy(&outcome.stdout);
    let stderr = String::from_utf8_lossy(&outcome.stderr);
    let output = format!("{stdout}\n{stderr}");
    let limited = outcome.reason == StopReason::OutputLimit;
    let cancelled = outcome.reason == StopReason::Cancelled;
    let timed_out = outcome.reason == StopReason::TimedOut;
    let mut result = json!({
        "language":match spec.program() { Program::Go => "go", Program::Python => "python", Program::Node => "javascript", Program::Dotnet => "dotnet", _ => "rust" }, "scope":args.scope.as_deref().unwrap_or("."),
        "action":match args.action { Some(VerifyAction::Test) => "test", _ => "build" },
        "code":outcome.exit_code,
        "passed":outcome.reason == StopReason::Exited && outcome.exit_code == Some(0),
        "ok":outcome.reason == StopReason::Exited && outcome.exit_code == Some(0),
        "milliseconds":outcome.elapsed.as_millis() as u64,
        "limited":limited, "cancelled":cancelled, "timedOut":timed_out,
        "treeTerminationConfirmed":outcome.tree_termination_confirmed,
        "output":prefix(&output, 12000),
        "complete":!limited && !cancelled && !timed_out,
        "_complete":!limited && !cancelled && !timed_out,
        "verificationFreshness":"not_tracked",
        "workspaceObservation":observation,
    });
    if spec.program() == Program::Dotnet {
        result["verificationMode"] = json!(if matches!(args.action, Some(VerifyAction::Test)) {
            "dotnet_test"
        } else {
            "dotnet_build"
        });
        result["testCountVerified"] = json!(false);
        result["implicitRestoreEnabled"] = json!(false);
    }
    if spec.program() == Program::Node {
        result["verificationMode"] = json!(if matches!(args.action, Some(VerifyAction::Test)) {
            "node_test_file"
        } else {
            "node_syntax_check"
        });
        result["testCountVerified"] = json!(false);
    }
    if spec.program() == Program::Python {
        result["verificationMode"] = json!(if matches!(args.action, Some(VerifyAction::Test)) {
            "unittest_discovery"
        } else {
            "py_compile_syntax_only"
        });
        result["testCountVerified"] = json!(false);
    }
    if outcome.reason == StopReason::Exited && outcome.exit_code != Some(0) {
        result["diagnostics"] = json!([{"code":"VERIFY_FAILED","message":"Verification tool returned a nonzero exit status; inspect output"}]);
    } else if limited {
        result["diagnostics"] =
            json!([{"code":"OUTPUT_LIMIT","message":"Combined process output exceeded 2 MiB"}]);
    }
    if utf16_len(&output) > 12000 || limited {
        result["truncated"] = json!(true);
        result["_detail"] = json!({"stdout":stdout,"stderr":stderr,"complete":!limited});
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotnet_plan_requires_explicit_project_and_disables_restore_servers() {
        let root = tempfile::tempdir().unwrap();
        let project = Project::new(root.path()).unwrap();
        for scope in ["project with spaces.csproj", "solution with spaces.sln"] {
            std::fs::write(root.path().join(scope), "fixture").unwrap();
            for action in [VerifyAction::Build, VerifyAction::Test] {
                let test = matches!(action, VerifyAction::Test);
                let spec = plan(
                    &project,
                    &VerifyArgs {
                        language: "csharp".into(),
                        scope: Some(scope.into()),
                        action: Some(action),
                        session: None,
                    },
                )
                .unwrap();
                assert_eq!(spec.program(), Program::Dotnet);
                assert_eq!(spec.arguments()[0], if test { "test" } else { "build" });
                assert_eq!(spec.arguments()[1], format!("./{scope}"));
                for flag in [
                    "--no-restore",
                    "--disable-build-servers",
                    "-nodeReuse:false",
                    "-maxCpuCount:1",
                    "-p:UseSharedCompilation=false",
                ] {
                    assert!(spec.arguments().contains(&flag.into()));
                }
            }
        }
    }

    #[test]
    fn dotnet_rejects_implicit_directories_and_unapproved_project_types() {
        let root = tempfile::tempdir().unwrap();
        for file in ["a.cs", "a.vbproj", "a.slnx"] {
            std::fs::write(root.path().join(file), "fixture").unwrap();
        }
        let project = Project::new(root.path()).unwrap();
        for scope in [
            None,
            Some("."),
            Some("a.cs"),
            Some("a.vbproj"),
            Some("a.slnx"),
            Some("../a.csproj"),
            Some("--help"),
            Some("C:/a.sln"),
        ] {
            let args = VerifyArgs {
                language: "dotnet".into(),
                scope: scope.map(String::from),
                action: None,
                session: None,
            };
            assert!(plan(&project, &args).is_err());
        }
    }

    #[test]
    #[ignore = "requires installed .NET SDK; run explicitly"]
    fn real_dotnet_build_and_test_dispatch() {
        let root = tempfile::tempdir().unwrap();
        let runner = NativeProcessRunner::new(root.path()).unwrap();
        let version = runner
            .run(
                &ProcessSpec::new(
                    Program::Dotnet,
                    vec!["--version".into()],
                    Duration::from_secs(10),
                    4096,
                )
                .unwrap(),
                &OperationContext::default(),
            )
            .unwrap();
        assert_eq!(version.exit_code, Some(0));
        let major: u32 = std::str::from_utf8(&version.stdout)
            .unwrap()
            .trim()
            .split('.')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            major >= 7,
            "fixture requires SDK supporting --disable-build-servers"
        );
        let csproj = format!(
            "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net{major}.0</TargetFramework></PropertyGroup></Project>"
        );
        std::fs::write(root.path().join("fixture with spaces.csproj"), csproj).unwrap();
        std::fs::write(
            root.path().join("Fixture.cs"),
            "public static class Fixture { public static int Value => 4; }\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("NuGet.Config"),
            "<configuration><packageSources><clear/></packageSources></configuration>",
        )
        .unwrap();
        // Only fixture setup restores, from installed SDK packs, with no NuGet sources.
        // The published verifier never invokes restore.
        let restore = runner
            .run(
                &ProcessSpec::new(
                    Program::Dotnet,
                    vec![
                        "restore".into(),
                        "./fixture with spaces.csproj".into(),
                        "--configfile".into(),
                        "./NuGet.Config".into(),
                        "--disable-build-servers".into(),
                        "-p:NuGetAudit=false".into(),
                    ],
                    Duration::from_secs(30),
                    2 * 1024 * 1024,
                )
                .unwrap(),
                &OperationContext::default(),
            )
            .unwrap();
        assert_eq!(
            restore.exit_code,
            Some(0),
            "{}",
            String::from_utf8_lossy(&restore.stderr)
        );
        let app = super::super::application(root.path()).unwrap();
        let call = |scope: &str, action| {
            app.execute(harness_domain::contracts::Request::Verify(VerifyArgs {
                language: "dotnet".into(),
                scope: Some(scope.into()),
                action: Some(action),
                session: None,
            }))
        };
        let built = call("fixture with spaces.csproj", VerifyAction::Build);
        assert_eq!(built["state"], "ok", "{built}");
        assert_eq!(built["scope"]["language"], "dotnet");
        assert_eq!(built["data"]["implicitRestoreEnabled"], false);
        std::fs::write(root.path().join("Fixture.cs"), "invalid C sharp fixture").unwrap();
        assert_eq!(
            call("fixture with spaces.csproj", VerifyAction::Build)["state"],
            "error"
        );
        // Audit command dispatch and failure propagation without downloading a test framework.
        std::fs::write(root.path().join("dispatch.csproj"), "<Project><Target Name=\"VSTest\"><Message Importance=\"high\" Text=\"NATIVE_TEST_DISPATCH\"/></Target></Project>").unwrap();
        let tested = call("dispatch.csproj", VerifyAction::Test);
        assert_eq!(tested["state"], "ok", "{tested}");
        assert!(
            tested["data"]["output"]
                .as_str()
                .unwrap()
                .contains("NATIVE_TEST_DISPATCH")
        );
        assert_eq!(tested["data"]["testCountVerified"], false);
        std::fs::write(root.path().join("dispatch.csproj"), "<Project><Target Name=\"VSTest\"><Error Text=\"expected test dispatch failure\"/></Target></Project>").unwrap();
        let failed = call("dispatch.csproj", VerifyAction::Test);
        assert_eq!(failed["state"], "error", "{failed}");
        assert_eq!(failed["diagnostics"][0]["code"], "VERIFY_FAILED");
    }

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
                        language: "JS".into(),
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
                language: "javascript".into(),
                scope: scope.map(String::from),
                action: None,
                session: None,
            };
            assert!(plan(&project, &args).is_err());
        }
        for language in ["javascript", "typescript", "ts"] {
            let args = VerifyArgs {
                language: language.into(),
                scope: Some("a.ts".into()),
                action: None,
                session: None,
            };
            let spec = plan(&project, &args).unwrap();
            assert_eq!(spec.program(), Program::Node);
            assert_eq!(spec.arguments(), ["--no-warnings", "--check", "./a.ts"]);
        }
    }

    #[test]
    #[ignore = "requires installed Node.js with builtin test runner; run explicitly"]
    fn real_javascript_check_tests_and_failures() {
        super::super::process::executable(Program::Node)
            .expect("Node unavailable in absolute PATH");
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
                language: "javascript".into(),
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
        std::fs::write(root.path().join("fixture.test.cjs"), "const test=require('node:test'); test('bad',()=>{throw new Error('expected failure')});\n").unwrap();
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

    #[test]
    fn python_plans_use_isolated_stdlib_commands_and_validated_scopes() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("file with spaces.py"), "x=1\n").unwrap();
        std::fs::create_dir(root.path().join("tests with spaces")).unwrap();
        let project = Project::new(root.path()).unwrap();
        let mut args = VerifyArgs {
            language: "PYTHON".into(),
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
                language: "python".into(),
                scope: scope.map(String::from),
                action: None,
                session: None,
            };
            assert!(plan(&project, &args).is_err());
        }
        let args = VerifyArgs {
            language: "python".into(),
            scope: Some("a.py".into()),
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
                language: "python".into(),
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
                language: "GO".into(),
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
                language: "go".into(),
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
                language: "go".into(),
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
            language: "go".into(),
            scope: None,
            action: None,
            session: None,
        }));
        assert_eq!(result["state"], "error", "{result}");
        assert_eq!(result["data"]["passed"], false);
        assert_eq!(result["diagnostics"][0]["code"], "VERIFY_FAILED");
    }

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
            language: "rust".into(),
            scope: Some("crate with spaces".into()),
            action: Some(VerifyAction::Test),
            session: None,
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
}

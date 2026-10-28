#[test]
fn classic_aspnet_detection_and_solution_engine() {
    assert!(dotnet::classic_aspnet(
        "<Project><ProjectTypeGuids>{349C5851-65DF-11DA-9384-00065B846F21}</ProjectTypeGuids></Project>"
    ));
    assert!(dotnet::classic_aspnet(
        "<Import Project='$(VSToolsPath)\\WebApplications\\Microsoft.WebApplication.targets'/>"
    ));
    assert!(!dotnet::classic_aspnet(
        "<!-- <Import Project='Microsoft.WebApplication.targets'/> --><Project Sdk='Microsoft.NET.Sdk.Web'/>"
    ));
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("Web.csproj"),
        "<Import Project='Microsoft.WebApplication.targets'/>",
    )
    .unwrap();
    std::fs::write(
        root.path().join("Web.sln"),
        "Project(\"{guid}\") = \"Web\", \"Web.csproj\", \"{guid}\"\nEndProject",
    )
    .unwrap();
    let project = Project::new(root.path()).unwrap();
    let mut args = VerifyArgs {
        project: None,
        configuration: None,
        language: "dotnet".into(),
        scope: Some("Web.sln".into()),
        package: None,
        action: Some(VerifyAction::Build),
        engine: Some("auto".into()),
        warnings: None,
        timeout_seconds: None,
        session: None,
    };
    let spec = plan(&project, &args).unwrap();
    assert_eq!(spec.program(), Program::Msbuild);
    assert_eq!(spec.output_limit(), 32 * 1024 * 1024);
    assert!(spec.arguments().iter().any(|arg| arg == "/t:Build"));
    assert!(!spec.arguments().iter().any(|arg| arg.contains("Restore")));
    args.engine = Some("dotnet".into());
    assert_eq!(plan(&project, &args).unwrap_err().0, "VERIFY_ENGINE");
    args.engine = Some("invalid".into());
    assert!(matches!(
        background_plan(root.path(), &args, None),
        Err(("VERIFY_ENGINE", _))
    ));
    args.engine = Some("auto".into());
    args.warnings = Some("invalid".into());
    assert!(matches!(
        background_plan(root.path(), &args, None),
        Err(("VERIFY_WARNINGS", _))
    ));
    args.warnings = None;
    args.action = Some(VerifyAction::Test);
    assert_eq!(plan(&project, &args).unwrap_err().0, "VERIFY_ACTION");
}

#[test]
fn dotnet_warning_totals_deduplicate_and_preserve_unknown_baseline() {
    let root = tempfile::tempdir().unwrap();
    let project = Project::new(root.path()).unwrap();
    let mut args = VerifyArgs {
        project: None,
        configuration: None,
        language: "dotnet".into(),
        scope: Some("fixture.csproj".into()),
        package: None,
        action: None,
        engine: None,
        warnings: None,
        timeout_seconds: None,
        session: Some(uuid::Uuid::new_v4().to_string()),
    };
    let output = "File.cs(1,2): warning CS0001: old warning [fixture.csproj]\nFile.cs(1,2): warning CS0001: old warning [fixture.csproj]\n    742 Warning(s)\n    0 Error(s)";
    let context = OperationContext::default();
    let first = dotnet::dotnet_warning_summary(&project, &args, output, true, &context);
    assert_eq!(first["warnings"], 742);
    assert_eq!(first["observedWarnings"], 1);
    assert_eq!(first["errors"], 0);
    assert!(first["newWarnings"].is_null());
    let repeated = dotnet::dotnet_warning_summary(&project, &args, output, true, &context);
    assert_eq!(repeated["newWarnings"], 0);
    let added = format!("{output}\nOther.cs(2,3): warning CS0002: new warning");
    let next = dotnet::dotnet_warning_summary(&project, &args, &added, true, &context);
    assert_eq!(next["newWarnings"], 1);
    let partial = dotnet::dotnet_warning_summary(&project, &args, output, false, &context);
    assert!(partial["warnings"].is_null());
    assert!(partial["newWarnings"].is_null());
    args.session = None;
    assert!(
        dotnet::dotnet_warning_summary(&project, &args, output, true, &context)["newWarnings"]
            .is_null()
    );
    let files = std::collections::BTreeSet::from(["src/file.cs".to_owned()]);
    assert!(dotnet::warning_matches_file(
        "src\\file.cs(1,2): warning CS1: issue",
        root.path(),
        &files
    ));
    assert!(!dotnet::warning_matches_file(
        "other/file.cs(1,2): warning CS1: issue",
        root.path(),
        &files
    ));
}

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
                    project: None,
                    configuration: None,
                    engine: None,
                    warnings: None,
                    timeout_seconds: None,
                    language: "csharp".into(),
                    package: None,
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
            project: None,
            configuration: None,
            engine: None,
            warnings: None,
            timeout_seconds: None,
            language: "dotnet".into(),
            package: None,
            scope: scope.map(String::from),
            action: None,
            session: None,
        };
        assert!(plan(&project, &args).is_err());
    }
}

#[test]
fn fast_per_file_razor_cshtml_validates_directives_tags_csharp_and_scripts_without_msbuild() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("Views/Taller")).unwrap();
    let cshtml_path = root.path().join("Views/Taller/Itinerario.cshtml");
    let valid_cshtml = r#"@model IEnumerable<Web.Models.ItinerarioItem>
@{
    var title = "Itinerario de Taller";
    var total = Model.Count();
}
<div class="itinerario-panel" data-title="@title">
    <h2>@title (@total)</h2>
    <style>
        @@media screen and (max-width: 768px) {
            .itinerario-panel { padding: 8px; }
        }
    </style>
</div>
@section Scripts {
    <script>
        const endpoint = '@Url.Action("Listar", "Taller")';
        const initialTotal = @(total);
        function refreshItinerario() {
            return fetch(endpoint).then(r => r.json());
        }
    </script>
}
"#;
    std::fs::write(&cshtml_path, valid_cshtml).unwrap();
    let project = Project::new(root.path()).unwrap();

    for language in ["dotnet", "csharp", "razor", "cshtml"] {
        let args: VerifyArgs = serde_json::from_value(json!({
            "language": language,
            "scope": "Views/Taller/Itinerario.cshtml"
        }))
        .unwrap();
        let result = verify(&project, &args, &OperationContext::default()).unwrap();
        assert_eq!(result["passed"], true, "{result}");
        assert_eq!(result["language"], "razor");
        assert_eq!(result["verificationMode"], "razor_fast_per_file");
        assert_eq!(result["fastPerFile"], true);
        assert_eq!(result["fullCompilationSkipped"], true);
        assert!(result["command"].is_null());

        let validated: Vec<&str> = result["validated"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(validated.contains(&"razor-directives (Views/Taller/Itinerario.cshtml)"));
        assert!(validated.contains(&"razor-syntax-balance (Views/Taller/Itinerario.cshtml)"));
        assert!(validated.contains(&"razor-html-tag-balance (Views/Taller/Itinerario.cshtml)"));
        assert!(
            validated
                .iter()
                .any(|item| item.starts_with("razor-csharp-blocks (Views/Taller/Itinerario.cshtml:"))
        );
        assert!(
            validated
                .iter()
                .any(|item| item.starts_with("razor-embedded-scripts (Views/Taller/Itinerario.cshtml:"))
        );
        assert!(
            validated
                .iter()
                .any(|item| item.starts_with("razor-embedded-styles (Views/Taller/Itinerario.cshtml:"))
        );

        let pending: Vec<&str> = result["pending"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(pending.contains(&"dotnet-msbuild-compile (skipped for fast per-file check)"));
        assert!(pending.contains(&"csharp-model-and-helper-typecheck (skipped for fast per-file check)"));
        assert!(pending.contains(&"browser-runtime-verification"));
    }

    // Now introduce an unbalanced HTML tag and an inline <script> syntax error in Razor
    let invalid_cshtml = r#"@model IEnumerable<Web.Models.ItinerarioItem>
<div class="itinerario-panel">
    <span>Unclosed span
</div>
@section Scripts {
    <script>
        function brokenInlineJs( {
            return 1;
        }
    </script>
}
"#;
    std::fs::write(&cshtml_path, invalid_cshtml).unwrap();
    let args: VerifyArgs = serde_json::from_value(json!({
        "language": "dotnet",
        "scope": "Views/Taller/Itinerario.cshtml"
    }))
    .unwrap();
    let failed = verify(&project, &args, &OperationContext::default()).unwrap();
    assert_eq!(failed["passed"], false, "{failed}");
    assert_eq!(failed["ok"], false);
    let diag_codes: Vec<&str> = failed["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["code"].as_str())
        .collect();
    assert!(
        diag_codes.contains(&"RAZOR_HTML_UNBALANCED"),
        "expected RAZOR_HTML_UNBALANCED in {diag_codes:?}"
    );
    assert!(
        diag_codes.contains(&"RAZOR_INLINE_JS_SYNTAX"),
        "expected RAZOR_INLINE_JS_SYNTAX in {diag_codes:?}"
    );
}

#[test]
fn fast_per_file_multi_file_combines_js_and_cshtml_checks() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("Scripts")).unwrap();
    std::fs::create_dir_all(root.path().join("Views/Taller")).unwrap();
    std::fs::write(
        root.path().join("Scripts/app.js"),
        "const ok = true; export default ok;\n",
    )
    .unwrap();
    std::fs::write(
        root.path().join("Views/Taller/Index.cshtml"),
        "<div><p>Hola</p></div>\n",
    )
    .unwrap();
    let project = Project::new(root.path()).unwrap();
    let args: VerifyArgs = serde_json::from_value(json!({
        "language": "dotnet",
        "files": "Scripts/app.js, Views/Taller/Index.cshtml"
    }))
    .unwrap();
    let result = verify(&project, &args, &OperationContext::default()).unwrap();
    assert_eq!(result["passed"], true, "{result}");
    assert_eq!(result["language"], "multi_file");
    assert_eq!(result["verificationMode"], "fast_per_file_syntax");
    assert_eq!(
        result["filesChecked"],
        json!(["Scripts/app.js", "Views/Taller/Index.cshtml"])
    );
    let validated: Vec<&str> = result["validated"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(validated.contains(&"js-syntax (Scripts/app.js)"));
    assert!(validated.contains(&"razor-syntax-balance (Views/Taller/Index.cshtml)"));
    let pending: Vec<&str> = result["pending"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(pending.contains(&"dotnet-msbuild-compile (skipped for fast per-file check)"));
    assert!(pending.contains(&"browser-runtime-verification"));
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
            project: None,
            configuration: None,
            engine: None,
            warnings: None,
            timeout_seconds: None,
            language: "dotnet".into(),
            package: None,
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

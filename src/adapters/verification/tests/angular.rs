#[test]
fn angular_plan_uses_only_the_closed_official_compiler_driver() {
    let root = tempfile::tempdir().unwrap();
    let project = Project::new(root.path()).unwrap();
    let mut args: VerifyArgs = serde_json::from_value(json!({"language":"angular","scope":".","project":"web app; literal","configuration":"development"})).unwrap();
    let spec = plan(&project, &args).unwrap();
    assert!(matches!(
        select_strategy(&project, &args).unwrap(),
        VerificationStrategy::AngularCompiler(_)
    ));
    assert_eq!(spec.program(), Program::Node);
    assert_eq!(&spec.arguments()[..2], &["--input-type=module", "--eval"]);
    assert!(spec.arguments()[2].contains("compiler.performCompilation"));
    let request: Value = serde_json::from_str(spec.arguments().last().unwrap()).unwrap();
    assert_eq!(request["project"], "web app; literal");
    assert_eq!(request["configuration"], "development");
    assert_eq!(spec.timeout(), Duration::from_secs(60));
    let BackgroundVerificationPlan::Process(background) =
        background_plan(root.path(), &args, None).unwrap()
    else {
        panic!("Angular must invoke the official compiler")
    };
    assert_eq!(background.timeout_option(), None);
    assert_eq!(background.arguments(), spec.arguments());
    args.action = Some(VerifyAction::Test);
    assert!(matches!(plan(&project, &args), Err(("UNSUPPORTED", _))));
    args.action = Some(VerifyAction::Build);
    args.scope = Some("../outside".into());
    assert!(plan(&project, &args).is_err());
    args.scope = Some(".".into());
    args.language = "typescript".into();
    assert!(matches!(
        typescript::typescript_scope(&project, &args),
        Err(("VERIFY_OPTIONS", _))
    ));
}

#[test]
fn angular_report_cannot_turn_process_failure_into_success() {
    let report = json!({"harnessAngularVerification":1,"passed":true,"compilerVersion":"20.3.0","typeCheckPerformed":true,"templateTypeCheckPerformed":true,"strictTemplates":true,"diagnostics":[]});
    let parsed = angular::report(&format!("other output\n{report}\n")).unwrap();
    let mut failed = json!({"passed":false,"ok":false,"diagnostics":[{"code":"TIMEOUT"}]});
    angular::annotate(&mut failed, Some(&parsed));
    assert_eq!(failed["passed"], false);
    assert_eq!(failed["diagnostics"][0]["code"], "TIMEOUT");
    let mut malformed = json!({"passed":true,"ok":true});
    angular::annotate(&mut malformed, None);
    assert_eq!(malformed["ok"], false);
    assert_eq!(malformed["diagnostics"][0]["code"], "ANGULAR_PROTOCOL");
}

#[test]
#[ignore = "requires Node and official Angular dependencies in HARNESS_ANGULAR_FIXTURE; no packages are installed by the test"]
fn angular_official_compiler_detects_real_template_errors_without_emitting() {
    let dependencies = std::env::var_os("HARNESS_ANGULAR_FIXTURE").expect(
        "set HARNESS_ANGULAR_FIXTURE to a directory with installed official Angular packages",
    );
    let root = tempfile::tempdir_in(dependencies).unwrap();
    std::fs::write(root.path().join("angular.json"), r#"{
      // Angular CLI workspace config supports JSON comments.
      "projects":{"web":{"architect":{"build":{"options":{"tsConfig":"tsconfig.app.json"},"defaultConfiguration":"development","configurations":{"development":{}}}}}}
    }"#).unwrap();
    std::fs::write(root.path().join("tsconfig.base.json"), r#"{"compilerOptions":{"target":"ES2022","module":"ES2022","moduleResolution":"bundler","experimentalDecorators":true,"strict":true,"skipLibCheck":true,"types":[],"lib":["es2022","dom"]},"angularCompilerOptions":{"strictTemplates":true}}"#).unwrap();
    std::fs::write(root.path().join("tsconfig.app.json"), r#"{"extends":"./tsconfig.base.json","compilerOptions":{"incremental":true,"tsBuildInfoFile":"./probe.tsbuildinfo"},"files":["app.ts"]}"#).unwrap();
    std::fs::write(root.path().join("app.ts"), r#"import {Component,Input} from '@angular/core';
      @Component({selector:'child-cmp',standalone:true,template:''}) export class Child { @Input() label = ''; }
      @Component({selector:'test-app',standalone:true,imports:[Child],templateUrl:'./app.html'})
      export class App { title = 'ready'; count = 1; user: {name:string}|null = null; }
    "#).unwrap();
    let project = Project::new(root.path()).unwrap();
    let args: VerifyArgs =
        serde_json::from_value(json!({"language":"angular","action":"build","project":"web"}))
            .unwrap();
    for (html, expected_pass, expected_code) in [
        (
            "@if (title) {<child-cmp [label]=\"title\" />} {{ user?.name }}",
            true,
            None,
        ),
        ("{{ missing }}", false, Some("TS2339")),
        ("<child-cmp [label]=\"count\" />", false, Some("TS2322")),
        (
            "<button (click)=\"missingMethod()\">Click</button>",
            false,
            Some("TS2339"),
        ),
        ("<unknown-widget />", false, Some("NG8001")),
        ("{{ title + }}", false, Some("NG5002")),
    ] {
        std::fs::write(root.path().join("app.html"), html).unwrap();
        let result = verify(&project, &args, &OperationContext::default()).unwrap();
        assert_eq!(result["passed"], expected_pass, "{html}: {result}");
        assert_eq!(result["language"], "angular", "{result}");
        assert_eq!(result["compiler"], "@angular/compiler-cli");
        assert_eq!(result["strictTemplates"], true);
        assert_eq!(result["noEmit"], true);
        assert_eq!(result["templateTypeCheckPerformed"], true);
        if let Some(code) = expected_code {
            assert!(
                result["diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["code"] == code
                        && d["path"] == "app.html"
                        && d["line"].is_number()
                        && d["column"].is_number()),
                "{result}"
            );
        }
        assert!(!root.path().join("app.js").exists());
        assert!(!root.path().join("app.ngtypecheck.ts").exists());
        assert!(!root.path().join("probe.tsbuildinfo").exists());
        assert_eq!(result["workspaceObservation"]["status"], "unchanged");
    }
    for (extra, code) in [
        (json!({"project":"missing"}), "ANGULAR_PROJECT"),
        (
            json!({"project":"web","configuration":"missing"}),
            "ANGULAR_CONFIGURATION",
        ),
    ] {
        let mut value = json!({"language":"angular"});
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let args: VerifyArgs = serde_json::from_value(value).unwrap();
        let result = verify(&project, &args, &OperationContext::default()).unwrap();
        assert_eq!(result["passed"], false, "{result}");
        assert!(
            result["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["code"] == code),
            "{result}"
        );
    }
    // Explicit config selection works without angular.json and reports inherited options.
    std::fs::write(root.path().join("app.html"), "{{title}}").unwrap();
    std::fs::remove_file(root.path().join("angular.json")).unwrap();
    let explicit: VerifyArgs =
        serde_json::from_value(json!({"language":"angular","scope":"tsconfig.app.json"})).unwrap();
    let result = verify(&project, &explicit, &OperationContext::default()).unwrap();
    assert_eq!(result["passed"], true, "{result}");
    std::fs::write(root.path().join("tsconfig.app.json"), r#"{"extends":"./tsconfig.base.json","files":["app.ts"],"angularCompilerOptions":{"strictTemplates":false}}"#).unwrap();
    let result = verify(&project, &explicit, &OperationContext::default()).unwrap();
    assert_eq!(result["passed"], true, "{result}");
    assert_eq!(result["strictTemplates"], false);
    assert!(
        result["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "ANGULAR_STRICT_TEMPLATES")
    );
}

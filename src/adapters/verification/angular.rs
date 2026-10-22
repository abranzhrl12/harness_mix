//! Invoke the project's installed official Angular compiler, without npm/npx or a shell.
use super::Project;
use harness_domain::{
    Failure,
    contracts::{VerifyAction, VerifyArgs},
    process::{ProcessSpec, Program},
};
use serde_json::Value;
use std::time::Duration;

pub(super) fn angular_plan(project: &Project, args: &VerifyArgs) -> Result<ProcessSpec, Failure> {
    if matches!(args.action, Some(VerifyAction::Test)) {
        return Err((
            "UNSUPPORTED",
            "Angular verification compiles TypeScript and templates; test execution is not supported",
        ));
    }
    let scope = args.scope.as_deref().unwrap_or(".");
    if scope.len() > 4096 {
        return Err(("PATH", "Angular scope is too long"));
    }
    project.target(scope)?;
    for selector in [&args.project, &args.configuration].into_iter().flatten() {
        if selector.is_empty() || selector.len() > 256 || selector.contains(['\0', '\r', '\n']) {
            return Err((
                "VERIFY_OPTIONS",
                "Invalid Angular project/configuration selector",
            ));
        }
    }
    ProcessSpec::new(
        Program::Node,
        vec![
            "--input-type=module".into(),
            "--eval".into(),
            include_str!("angular_driver.mjs").into(),
            "--".into(),
            serde_json::json!({"scope":scope,"project":args.project,"configuration":args.configuration}).to_string(),
        ],
        Duration::from_secs(60),
        2 * 1024 * 1024,
    )
}

pub(in crate::adapters) fn report(stdout: &str) -> Option<Value> {
    stdout.lines().rev().find_map(|line| {
        let report: Value = serde_json::from_str(line).ok()?;
        (report["harnessAngularVerification"] == 1).then_some(report)
    })
}

pub(in crate::adapters) fn display(report: &Value) -> String {
    report["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|d| {
            let location = d["path"]
                .as_str()
                .map(|path| format!("{path}:{}:{}: ", d["line"], d["column"]))
                .unwrap_or_default();
            format!(
                "{location}[{}] {}",
                d["code"].as_str().unwrap_or("ANGULAR"),
                d["message"].as_str().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(in crate::adapters) fn annotate(result: &mut Value, report: Option<&Value>) {
    result["language"] = serde_json::json!("angular");
    result["verificationMode"] = serde_json::json!("angular_official_compiler");
    result["compiler"] = serde_json::json!("@angular/compiler-cli");
    result["noEmit"] = serde_json::json!(true);
    result["bundleBuildPerformed"] = serde_json::json!(false);
    result["testCountVerified"] = serde_json::json!(false);
    result["verificationAvailable"] = serde_json::json!(false);
    result["typeCheckPerformed"] = serde_json::json!(false);
    result["templateTypeCheckPerformed"] = serde_json::json!(false);
    if let Some(report) = report {
        for key in [
            "compilerVersion",
            "verificationAvailable",
            "typeCheckPerformed",
            "templateTypeCheckPerformed",
            "strictTemplates",
            "tsconfig",
            "project",
            "configuration",
        ] {
            if let Some(value) = report.get(key) {
                result[key] = value.clone();
            }
        }
        let passed = result["passed"] == true && report["passed"] == true;
        result["passed"] = serde_json::json!(passed);
        result["ok"] = serde_json::json!(passed);
        if let Some(diagnostics) = report["diagnostics"].as_array() {
            let existing = result
                .as_object_mut()
                .unwrap()
                .entry("diagnostics")
                .or_insert_with(|| serde_json::json!([]));
            if let Some(items) = existing.as_array_mut() {
                items.extend(diagnostics.iter().cloned());
            }
        }
    } else if result["passed"] == true {
        result["passed"] = serde_json::json!(false);
        result["ok"] = serde_json::json!(false);
        result["diagnostics"] = serde_json::json!([{"code":"ANGULAR_PROTOCOL","message":"Angular process returned no compiler report","severity":"error"}]);
    }
}

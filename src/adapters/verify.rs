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
    if !args.language.eq_ignore_ascii_case("rust") {
        return Err(("UNSUPPORTED", "Only Rust verification is migrated"));
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
    // A failed observation must not hide Cargo's actual result or pretend freshness.
    let after = super::fingerprint::capture(project, context);
    let mut observation = match (&before, &after) {
        (Ok(a), Ok(b)) => {
            json!({"status":if a == b { "unchanged" } else { "changed" }, "before":a,"after":b})
        }
        _ => {
            json!({"status":"unknown","beforeError":before.as_ref().err().map(|e| e.0),"afterError":after.as_ref().err().map(|e| e.0)})
        }
    };
    observation["coverage"] = json!({
        "kind":"local_workspace_before_after",
        "excludedDirectories":["target","node_modules",".git",".hg",".svn",".agents/harness/.state",".agents/harness/logs"],
        "externalDependenciesTracked":false,
        "environmentTracked":false,
        "current":null
    });
    let stdout = String::from_utf8_lossy(&outcome.stdout);
    let stderr = String::from_utf8_lossy(&outcome.stderr);
    let output = format!("{stdout}\n{stderr}");
    let limited = outcome.reason == StopReason::OutputLimit;
    let cancelled = outcome.reason == StopReason::Cancelled;
    let timed_out = outcome.reason == StopReason::TimedOut;
    let mut result = json!({
        "language":"rust", "scope":args.scope.as_deref().unwrap_or("."),
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
    if outcome.reason == StopReason::Exited && outcome.exit_code != Some(0) {
        result["diagnostics"] = json!([{"code":"VERIFY_FAILED","message":"Cargo returned a nonzero exit status; inspect output"}]);
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
        args.language = "javascript".into();
        assert_eq!(plan(&project, &args).unwrap_err().0, "UNSUPPORTED");
    }
}

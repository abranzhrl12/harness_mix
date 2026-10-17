//! Read-only, bounded local diagnostics. No repair, build, index sync or configuration output.
use super::{
    process::{NativeProcessRunner, executable},
    project::Project,
};
use harness_application::ports::ProcessRunner;
use harness_domain::{
    Failure,
    contracts::DoctorArgs,
    execution::OperationContext,
    process::{ProcessSpec, Program, StopReason},
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn check(id: &str, status: &str, message: &str) -> Value {
    json!({"id":id,"status":status,"message":message})
}

// Only publish numeric version components, never raw stdout/stderr or tool paths.
fn numeric_version(program: Program, output: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(output).ok()?;
    let prefix = match program {
        Program::Cargo => "cargo ",
        Program::Rustc => "rustc ",
        Program::Git => "git version ",
        Program::Go => "go version go",
    };
    let candidate = text.strip_prefix(prefix)?.split_whitespace().next()?;
    let numeric: String = candidate
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let numeric = numeric.trim_end_matches('.');
    let parts: Vec<_> = numeric.split('.').collect();
    if !(2..=4).contains(&parts.len()) || parts.iter().any(|s| s.is_empty() || s.len() > 5) {
        return None;
    }
    Some(numeric.to_owned())
}

fn tool_check(
    program: Program,
    name: &str,
    runner: Option<&NativeProcessRunner>,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    let id = format!("tool:{name}");
    if executable(program).is_err() {
        let mut result = check(
            &id,
            "warning",
            "Optional executable unavailable in absolute PATH entries",
        );
        result["located"] = json!(false);
        result["versionChecked"] = json!(false);
        return Ok(result);
    }
    let mut result = check(&id, "ok", "Executable located; not executed");
    result["located"] = json!(true);
    result["versionChecked"] = json!(false);
    if let Some(runner) = runner {
        let spec = ProcessSpec::new(
            program,
            vec![
                if program == Program::Go {
                    "version"
                } else {
                    "--version"
                }
                .into(),
            ],
            Duration::from_secs(2),
            4096,
        )?;
        match runner.run(&spec, context) {
            Ok(outcome) => {
                context.check()?;
                let version = (outcome.reason == StopReason::Exited
                    && outcome.exit_code == Some(0))
                .then(|| numeric_version(program, &outcome.stdout))
                .flatten();
                if let Some(version) = version {
                    result["versionChecked"] = json!(true);
                    result["numericVersion"] = json!(version);
                    result["message"] = json!("Bounded version probe completed");
                } else {
                    result["status"] = json!("warning");
                    result["message"] = json!(
                        "Version probe failed, exceeded its budget, or returned unrecognized output"
                    );
                    result["probeCode"] = json!(match outcome.reason {
                        StopReason::TimedOut => "TIMEOUT",
                        StopReason::Cancelled => "CANCELLED",
                        StopReason::OutputLimit => "OUTPUT_LIMIT",
                        StopReason::Exited => "VERSION_UNRECOGNIZED",
                    });
                }
            }
            Err((code, _)) => {
                context.check()?;
                result["status"] = json!("warning");
                result["message"] = json!("Version probe could not complete");
                result["probeCode"] = json!(code);
            }
        }
    }
    Ok(result)
}

fn path_check(project: &Project, path: &str, id: &str, directory_message: &str) -> Value {
    match project.target_optional(path) {
        Ok(path) if path.is_dir() => check(id, "ok", directory_message),
        Ok(path) if !path.exists() => {
            check(id, "skipped", "Directory absent; no files were created")
        }
        Ok(_) => check(id, "error", "Expected a directory; no changes made"),
        Err(_) => check(
            id,
            "error",
            "Path inaccessible or restricted; no changes made",
        ),
    }
}

fn counts(checks: &[Value]) -> (Value, &'static str) {
    let mut counts = json!({"ok":0,"warning":0,"error":0,"skipped":0});
    for item in checks {
        let status = item["status"].as_str().unwrap_or("error");
        counts[status] = json!(counts[status].as_u64().unwrap_or(0) + 1);
    }
    let health = if counts["error"].as_u64().unwrap_or(0) > 0 {
        "error"
    } else if counts["warning"].as_u64().unwrap_or(0) > 0 {
        "warning"
    } else {
        "ok"
    };
    (counts, health)
}

pub(super) fn doctor(
    project: &Project,
    args: &DoctorArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    let started = Instant::now();
    let versions = args.versions.unwrap_or(false);
    let mut checks = vec![check(
        "runtime",
        "ok",
        "Native Rust MCP runtime; Node not required for implemented tools",
    )];
    checks[0]["version"] = json!(env!("CARGO_PKG_VERSION"));
    checks[0]["platform"] = json!({"os":std::env::consts::OS,"arch":std::env::consts::ARCH});
    checks.push(check(
        "workspace",
        if project.root.is_dir() { "ok" } else { "error" },
        "Workspace directory existence only; not a writeability or security audit",
    ));
    let runner = if versions && project.root.is_dir() {
        Some(NativeProcessRunner::new(&project.root)?)
    } else {
        None
    };
    for (program, name) in [
        (Program::Cargo, "cargo"),
        (Program::Rustc, "rustc"),
        (Program::Git, "git"),
        (Program::Go, "go"),
    ] {
        checks.push(tool_check(program, name, runner.as_ref(), context)?);
    }
    checks.push(path_check(
        project,
        ".agents/harness/.state",
        "session-storage",
        "Session directory path validated; contents and writeability not tested",
    ));
    let mut index = path_check(
        project,
        ".codegraph",
        "index",
        "Index directory exists, but its contents/freshness are not validated",
    );
    if index["status"] == "ok" {
        index["status"] = json!("warning");
    }
    checks.push(index);
    let mut connection = check(
        "connection:codegraph",
        "skipped",
        "Native CodeGraph connection diagnostic not migrated; no connection attempted",
    );
    connection["requested"] = json!(args.connection.unwrap_or(false));
    checks.push(connection);
    checks.push(check(
        "sql",
        "skipped",
        "Native SQL diagnostic not migrated; no configuration read or connection attempted",
    ));
    context.check()?;
    let (counts, health) = counts(&checks);
    Ok(
        json!({"health":health,"counts":counts,"checks":checks,"milliseconds":started.elapsed().as_millis() as u64,
        "coverage":"native_local_only","versionsRequested":versions,
        "limitations":["No builds, tests, repairs, index sync, or configuration/secret reads",
            "Versions default false; true runs bounded version probes (Go: version; others: --version), 2 seconds and 4 KiB each",
            "Toolchain shims may consult their manager/project selection; version probing is not a network sandbox",
            "Executable discovery uses the same absolute PATH/direct executable policy as native execution",
            "Directory presence does not validate contents, permissions, freshness, or remote transport health",
            "CodeGraph and SQL connections remain unverified; application attaches live metrics and volatile log status"]}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn version_output_never_forwards_raw_text() {
        assert_eq!(
            numeric_version(
                Program::Go,
                b"go version go1.25.2 windows/amd64 private-suffix"
            ),
            Some("1.25.2".into())
        );
        assert!(numeric_version(Program::Go, b"go version private-token").is_none());
        assert_eq!(
            numeric_version(Program::Git, b"git version 2.51.0.windows.1\n"),
            Some("2.51.0".into())
        );
        assert_eq!(
            numeric_version(
                Program::Rustc,
                b"rustc 1.94.0-nightly (sensitive extra text)"
            ),
            Some("1.94.0".into())
        );
        assert!(numeric_version(Program::Cargo, b"credential=not-a-version").is_none());
        assert!(numeric_version(Program::Cargo, b"cargo 1..3 secret").is_none());
    }
    #[test]
    fn skipped_is_not_a_successful_connection_and_errors_dominate_health() {
        let (summary, health) = counts(&[check("a", "warning", ""), check("b", "skipped", "")]);
        assert_eq!(health, "warning");
        assert_eq!(summary["ok"], 0);
        assert_eq!(
            counts(&[check("a", "error", ""), check("b", "warning", "")]).1,
            "error"
        );
    }
}

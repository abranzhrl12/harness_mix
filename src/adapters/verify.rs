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

pub(super) enum BackgroundVerificationPlan {
    Process(ProcessSpec),
    TypeScriptSyntax,
}

/// A verifier selected from the language-specific capabilities available in the workspace.
/// Process-backed strategies carry their already validated invocation; fast per-file and
/// TypeScript source checks remain in-process and never execute project code.
enum VerificationStrategy {
    TypeScriptSyntax,
    AngularCompiler(ProcessSpec),
    Process(ProcessSpec),
}

impl VerificationStrategy {
    fn process_spec(&self) -> Option<&ProcessSpec> {
        match self {
            Self::TypeScriptSyntax => None,
            Self::AngularCompiler(spec) | Self::Process(spec) => Some(spec),
        }
    }

    fn is_angular(&self) -> bool {
        matches!(self, Self::AngularCompiler(_))
    }

    fn into_process_spec(self) -> Option<ProcessSpec> {
        match self {
            Self::TypeScriptSyntax => None,
            Self::AngularCompiler(spec) | Self::Process(spec) => Some(spec),
        }
    }
}

#[path = "verification/rust.rs"]
mod rust;

#[path = "verification/typescript.rs"]
mod typescript;

#[path = "verification/angular.rs"]
pub(in crate::adapters) mod angular;

pub(super) use typescript::run_typescript_syntax;

fn validate_verify_options(args: &VerifyArgs) -> Result<(), Failure> {
    args.effective_timeout()?;
    if (args.project.is_some() || args.configuration.is_some())
        && !args.language.eq_ignore_ascii_case("angular")
    {
        return Err((
            "VERIFY_OPTIONS",
            "project and configuration are supported only for Angular",
        ));
    }
    let engine = args.engine.as_deref().unwrap_or("auto");
    if !matches!(engine, "auto" | "dotnet" | "msbuild" | "fast" | "syntax") {
        return Err((
            "VERIFY_ENGINE",
            "engine must be auto, dotnet, msbuild, fast or syntax",
        ));
    }
    if !matches!(
        args.warnings.as_deref().unwrap_or("all"),
        "all" | "changed-files" | "none"
    ) {
        return Err((
            "VERIFY_WARNINGS",
            "warnings must be all, changed-files or none",
        ));
    }
    if matches!(args.engine.as_deref(), Some("dotnet" | "msbuild"))
        && !args.language.eq_ignore_ascii_case("dotnet")
        && !args.language.eq_ignore_ascii_case("csharp")
    {
        return Err((
            "VERIFY_OPTIONS",
            "dotnet and msbuild engines are supported only for .NET",
        ));
    }
    if args.warnings.is_some()
        && !args.language.eq_ignore_ascii_case("dotnet")
        && !args.language.eq_ignore_ascii_case("csharp")
    {
        return Err((
            "VERIFY_OPTIONS",
            "warnings filter is supported only for .NET",
        ));
    }
    Ok(())
}

fn plan(project: &Project, args: &VerifyArgs) -> Result<ProcessSpec, Failure> {
    if let Some(package) = &args.package {
        if !args.language.eq_ignore_ascii_case("rust")
            || package.is_empty()
            || package.starts_with('-')
            || package.len() > 128
            || !package
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err((
                "VERIFY_PACKAGE",
                "package must be a Rust package name (letters, digits, underscore or hyphen)",
            ));
        }
    }
    if args.language.eq_ignore_ascii_case("angular") {
        return angular::angular_plan(project, args);
    }
    if args.language.eq_ignore_ascii_case("dotnet") || args.language.eq_ignore_ascii_case("csharp")
    {
        return dotnet::dotnet_plan(project, args);
    }
    if args.language.eq_ignore_ascii_case("javascript")
        || args.language.eq_ignore_ascii_case("js")
        || args.language.eq_ignore_ascii_case("typescript")
        || args.language.eq_ignore_ascii_case("ts")
    {
        return javascript::javascript_plan(project, args);
    }
    if args.language.eq_ignore_ascii_case("python") {
        return python::python_plan(project, args);
    }
    if args.language.eq_ignore_ascii_case("go") {
        return go::go_plan(project, args);
    }
    if ["razor", "cshtml", "css", "scss", "vue", "astro"]
        .iter()
        .any(|lang| args.language.eq_ignore_ascii_case(lang))
    {
        return Err((
            "VERIFY_SCOPE",
            "Razor/CSS/SCSS/Vue/Astro verification uses fast in-process per-file checks; pass an explicit file path in scope",
        ));
    }
    if !args.language.eq_ignore_ascii_case("rust") {
        return Err((
            "UNSUPPORTED",
            "Supported verifiers: Rust, Go, Python, JavaScript, TypeScript, Razor, CSS/SCSS, Vue, Astro, Angular and dotnet",
        ));
    }
    rust::plan(project, args)
}

fn select_strategy(
    project: &Project,
    args: &VerifyArgs,
) -> Result<VerificationStrategy, Failure> {
    if typescript::typescript_scope(project, args)?.is_some() {
        return Ok(VerificationStrategy::TypeScriptSyntax);
    }
    let spec = plan(project, args)?;
    if args.language.eq_ignore_ascii_case("angular") {
        Ok(VerificationStrategy::AngularCompiler(spec))
    } else {
        Ok(VerificationStrategy::Process(spec))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VerificationStatus {
    passed: bool,
    limited: bool,
    cancelled: bool,
    timed_out: bool,
}

impl VerificationStatus {
    fn interpret(outcome: &harness_domain::process::ProcessOutcome) -> Self {
        Self {
            passed: outcome.reason == StopReason::Exited && outcome.exit_code == Some(0),
            limited: outcome.reason == StopReason::OutputLimit,
            cancelled: outcome.reason == StopReason::Cancelled,
            timed_out: outcome.reason == StopReason::TimedOut,
        }
    }

    fn complete(self) -> bool {
        !self.limited && !self.cancelled && !self.timed_out
    }
}

#[cfg(test)]
fn verification_plan(project: &Project, args: &VerifyArgs) -> Result<ProcessSpec, Failure> {
    verification_plan_from(plan(project, args)?, args)
}

fn verification_plan_from(spec: ProcessSpec, args: &VerifyArgs) -> Result<ProcessSpec, Failure> {
    let arguments = spec
        .arguments()
        .iter()
        .map(|argument| {
            if spec.program() == Program::Go && argument == "-timeout=20s" {
                args.timeout_seconds
                    .map(|seconds| format!("-timeout={seconds}s"))
                    .unwrap_or_else(|| argument.clone())
            } else {
                argument.clone()
            }
        })
        .collect();
    ProcessSpec::new(
        spec.program(),
        arguments,
        Duration::from_secs(30),
        spec.output_limit(),
    )?
    .with_verification_timeout(args.effective_timeout()?)
}

#[path = "verification/dotnet.rs"]
mod dotnet;

#[path = "verification/javascript.rs"]
mod javascript;

#[path = "verification/python.rs"]
mod python;

#[path = "verification/go.rs"]
mod go;

/// Reuse the closed verifier commands and workspace validation for managed jobs.
/// The per-call budget is deliberately replaced by the job's optional timeout.
pub(super) fn background_plan(
    root: &std::path::Path,
    args: &VerifyArgs,
    timeout: Option<Duration>,
) -> Result<BackgroundVerificationPlan, Failure> {
    if timeout.is_some_and(|value| value.is_zero() || value > Duration::from_secs(86400)) {
        return Err((
            "PROCESS_BUDGET",
            "Background timeout must be 1 second to 24 hours",
        ));
    }
    validate_verify_options(args)?;
    if args.session.is_some() {
        return Err((
            "ARGUMENT",
            "Background verification does not attach session freshness; omit session",
        ));
    }
    let project = Project::new(root).map_err(|_| ("PATH", "Workspace unavailable"))?;
    let strategy = select_strategy(&project, args)?;
    let Some(spec) = strategy.process_spec() else {
        return Ok(BackgroundVerificationPlan::TypeScriptSyntax);
    };
    let arguments = spec
        .arguments()
        .iter()
        .map(|argument| {
            if spec.program() == Program::Go && argument == "-timeout=20s" {
                // Zero disables Go's own default test deadline. The job manager
                // handles an explicitly requested execution timeout for the tree.
                "-timeout=0".to_owned()
            } else {
                argument.clone()
            }
        })
        .collect();
    let verifier_timeout = args.effective_timeout()?;
    let execution_timeout = timeout.or_else(|| args.timeout_seconds.map(Duration::from_secs));
    ProcessSpec::new(
        spec.program(),
        arguments,
        Duration::from_secs(30),
        spec.output_limit(),
    )?
    .with_verification_timeout(verifier_timeout)?
    .with_background_timeout(execution_timeout)
    .map(BackgroundVerificationPlan::Process)
}

pub(super) fn verify(
    project: &Project,
    args: &VerifyArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    if !matches!(
        args.warnings.as_deref().unwrap_or("all"),
        "all" | "changed-files" | "none"
    ) {
        return Err((
            "VERIFY_WARNINGS",
            "warnings must be all, changed-files or none",
        ));
    }
    validate_verify_options(args)?;
    let strategy = select_strategy(project, args)?;
    let angular = strategy.is_angular();
    let spec = strategy
        .into_process_spec()
        .map(|spec| verification_plan_from(spec, args))
        .transpose()?;
    let native_per_file = spec.is_none();
    let before = super::fingerprint::capture(project, context);
    context.check()?;
    let (outcome, per_file_report) = if native_per_file {
        let report = typescript::run_per_file_verification(project, args, context)?;
        let outcome = report.outcome.clone();
        (outcome, Some(report))
    } else {
        let outcome = NativeProcessRunner::new(&project.root)?.run(spec.as_ref().unwrap(), context)?;
        (outcome, None)
    };
    let program = spec.as_ref().map(ProcessSpec::program);
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
    let raw_stdout = String::from_utf8_lossy(&outcome.stdout);
    let angular_report = angular.then(|| angular::report(&raw_stdout)).flatten();
    let stdout = angular_report
        .as_ref()
        .map(angular::display)
        .map(std::borrow::Cow::Owned)
        .unwrap_or_else(|| raw_stdout.clone());
    let stderr = String::from_utf8_lossy(&outcome.stderr);
    let output = if stderr.is_empty() {
        stdout.to_string()
    } else if stdout.is_empty() {
        stderr.to_string()
    } else {
        format!("{stdout}\n{stderr}")
    };
    let status = VerificationStatus::interpret(&outcome);
    let limited = status.limited;
    let cancelled = status.cancelled;
    let timed_out = status.timed_out;
    let resolved_language = if let Some(report) = &per_file_report {
        report.language.as_str()
    } else {
        match program {
            None => "typescript",
            Some(Program::Go) => "go",
            Some(Program::Python) => "python",
            Some(Program::Node) => "javascript",
            Some(Program::Dotnet | Program::Msbuild) => "dotnet",
            _ => "rust",
        }
    };
    let mut result = json!({
        "language":resolved_language,
        "scope":args.scope.as_deref().unwrap_or("."),
        "action":match args.action { Some(VerifyAction::Test) => "test", None if program == Some(Program::Cargo) => "check", _ => "build" },
        "package":args.package,
        "command":spec.as_ref().map(|spec| json!({"program":format!("{:?}",spec.program()).to_ascii_lowercase(),"arguments":spec.arguments()})),
        "code":outcome.exit_code,
        "passed":status.passed,
        "ok":status.passed,
        "milliseconds":outcome.elapsed.as_millis() as u64,
        "limited":limited, "cancelled":cancelled, "timedOut":timed_out,
        "treeTerminationConfirmed":outcome.tree_termination_confirmed,
        "output":prefix(&output, 12000),
        "stdoutTail":stdout.chars().rev().take(4000).collect::<String>().chars().rev().collect::<String>(),
        "stderrTail":stderr.chars().rev().take(4000).collect::<String>().chars().rev().collect::<String>(),
        "complete":status.complete(),
        "_complete":status.complete(),
        "verificationFreshness":"not_tracked",
        "workspaceObservation":observation,
    });
    if let Some(report) = &per_file_report {
        result["verificationMode"] = json!(report.verification_mode);
        result["fastPerFile"] = json!(true);
        result["fullCompilationSkipped"] = json!(true);
        result["typeCheckPerformed"] = json!(false);
        result["codeExecuted"] = json!(false);
        result["testCountVerified"] = json!(false);
        result["filesChecked"] = json!(report.files_checked);
        result["validated"] = json!(report.validated);
        result["pending"] = json!(report.pending);
        result["summary"] = json!(report.summary);
        let mut diag_items = report.diagnostics.clone();
        if !status.passed {
            diag_items.insert(
                0,
                json!({
                    "code": "VERIFY_FAILED",
                    "message": format!("Fast per-file verification failed: {}", report.summary),
                    "severity": "error"
                }),
            );
        }
        if !diag_items.is_empty() {
            result["diagnostics"] = json!(diag_items);
        }
    } else {
        let (validated, pending, summary) =
            typescript::process_verification_transparency(program, angular, args, status.passed);
        result["fastPerFile"] = json!(false);
        result["validated"] = json!(validated);
        result["pending"] = json!(pending);
        result["summary"] = json!(summary);
    }
    if matches!(program, Some(Program::Dotnet | Program::Msbuild)) {
        let summary = dotnet::dotnet_warning_summary(
            project,
            args,
            &output,
            !limited && !cancelled && !timed_out,
            context,
        );
        for (key, value) in summary.as_object().unwrap() {
            result[key] = value.clone();
        }
        result["engine"] = json!(if program == Some(Program::Msbuild) {
            "msbuild"
        } else {
            "dotnet"
        });
        result["warningsFilter"] = json!(args.warnings.as_deref().unwrap_or("all"));
        if matches!(args.warnings.as_deref(), Some("changed-files" | "none")) {
            let selected: Vec<String> = result["changedFilesWarnings"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect();
            let filter = |text: &str| {
                text.lines()
                    .filter(|line| {
                        dotnet::diagnostic_lines(line, "warning").is_empty()
                            || (args.warnings.as_deref() == Some("changed-files")
                                && selected.iter().any(|selected| selected == line.trim()))
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let display_output = filter(&output);
            result["output"] = json!(prefix(&display_output, 12000));
            result["stdoutTail"] = json!(prefix(&filter(&stdout), 4000));
            result["stderrTail"] = json!(prefix(&filter(&stderr), 4000));
        }
        result["verificationMode"] = json!(if matches!(args.action, Some(VerifyAction::Test)) {
            "dotnet_test"
        } else {
            "dotnet_build"
        });
        result["testCountVerified"] = json!(false);
        result["implicitRestoreEnabled"] = json!(false);
    }
    if program == Some(Program::Node) && !angular {
        result["verificationMode"] = json!(if matches!(args.action, Some(VerifyAction::Test)) {
            "node_test_file"
        } else {
            "node_syntax_check"
        });
        result["testCountVerified"] = json!(false);
    }
    if program == Some(Program::Python) {
        result["verificationMode"] = json!(if matches!(args.action, Some(VerifyAction::Test)) {
            "unittest_discovery"
        } else {
            "py_compile_syntax_only"
        });
        result["testCountVerified"] = json!(false);
    }
    if !native_per_file {
        if outcome.reason == StopReason::Exited && outcome.exit_code != Some(0) {
            result["diagnostics"] = json!([{"code":"VERIFY_FAILED","message":"Verification tool returned a nonzero exit status; inspect output"}]);
        } else if timed_out {
            result["diagnostics"] = json!([{"code":"TIMEOUT","message":"Verification deadline reached; inspect command and captured output tails"}]);
        } else if cancelled {
            result["diagnostics"] = json!([{"code":"CANCELLED","message":"Verification cancelled; captured output may be partial"}]);
        } else if limited {
            result["diagnostics"] = json!([{"code":"OUTPUT_LIMIT","message":"Combined process output exceeded the bounded capture budget","limitBytes":spec.as_ref().map(ProcessSpec::output_limit)}]);
        }
    }
    if angular {
        angular::annotate(&mut result, angular_report.as_ref());
        let (validated, pending, summary) = typescript::process_verification_transparency(
            program,
            true,
            args,
            result["passed"] == true,
        );
        result["validated"] = json!(validated);
        result["pending"] = json!(pending);
        result["summary"] = json!(summary);
        // Expose the invocation contract without echoing the embedded driver source.
        result["command"] = json!({"program":"node","mode":"embedded_module","compiler":"@angular/compiler-cli","scope":args.scope.as_deref().unwrap_or("."),"project":args.project,"configuration":args.configuration,"noEmit":true});
    }
    if utf16_len(&output) > 12000 || limited {
        result["truncated"] = json!(true);
        result["_detail"] = json!({"stdout":stdout,"stderr":stderr,"complete":!limited});
    }
    Ok(result)
}

#[cfg(test)]
#[path = "verification/tests.rs"]
mod tests;

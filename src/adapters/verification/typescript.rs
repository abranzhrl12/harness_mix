use super::{Project, validate_verify_options};
use harness_domain::{
    Failure,
    contracts::{VerifyAction, VerifyArgs},
    execution::OperationContext,
    process::{ProcessOutcome, StopReason},
};
use std::time::Instant;

pub(super) fn typescript_scope(
    project: &Project,
    args: &VerifyArgs,
) -> Result<Option<String>, Failure> {
    validate_verify_options(args)?;
    if args.package.is_some() && !args.language.eq_ignore_ascii_case("rust") {
        return Err(("VERIFY_PACKAGE", "package is supported only for Rust"));
    }
    if !["javascript", "js", "typescript", "ts"]
        .iter()
        .any(|language| args.language.eq_ignore_ascii_case(language))
    {
        return Ok(None);
    }
    let Some(scope) = args.scope.as_deref() else {
        return Ok(None);
    };
    if !std::path::Path::new(scope)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| ["ts", "tsx"].contains(&extension))
    {
        return Ok(None);
    }
    if scope.len() > 4096 {
        return Err(("PATH", "Scope too long"));
    }
    if !project.target(scope)?.is_file() {
        return Err((
            "VERIFY_SCOPE",
            "TypeScript scope must be an explicit .ts or .tsx file",
        ));
    }
    if matches!(args.action, Some(VerifyAction::Test)) {
        return Err((
            "UNSUPPORTED",
            "TypeScript/TSX test execution is not supported; build performs native syntax validation only",
        ));
    }
    Ok(Some(scope.to_owned()))
}

pub(in crate::adapters) fn run_typescript_syntax(
    root: &std::path::Path,
    args: &VerifyArgs,
    context: &OperationContext,
) -> Result<ProcessOutcome, Failure> {
    context.check()?;
    let started = Instant::now();
    let project = Project::new(root).map_err(|_| ("PATH", "Workspace unavailable"))?;
    let scope = typescript_scope(&project, args)?.ok_or((
        "VERIFY_SCOPE",
        "Expected an explicit TypeScript/TSX syntax check",
    ))?;
    let bytes = project.bytes_with_limit(&scope, context, 512 * 1024)?;
    let source =
        std::str::from_utf8(&bytes).map_err(|_| ("ENCODING", "TypeScript source must be UTF-8"))?;
    let syntax = super::super::ts_syntax::validate_syntax(&scope, source, context);
    let (exit_code, stderr) = match syntax {
        Ok(()) => (Some(0), vec![]),
        Err(("SYNTAX_ERROR", message)) => (Some(1), message.as_bytes().to_vec()),
        Err(error) => return Err(error),
    };
    context.check()?;
    Ok(ProcessOutcome {
        exit_code,
        stdout: vec![],
        stderr,
        elapsed: started.elapsed(),
        reason: StopReason::Exited,
        tree_termination_confirmed: true,
    })
}

use super::Project;
use harness_domain::{
    Failure,
    contracts::{VerifyAction, VerifyArgs},
    process::{ProcessSpec, Program},
};
use std::time::Duration;

pub(super) fn javascript_plan(
    project: &Project,
    args: &VerifyArgs,
) -> Result<ProcessSpec, Failure> {
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
    if is_ts {
        return Err((
            "UNSUPPORTED",
            "TypeScript builds use native syntax validation; TypeScript test execution is not supported",
        ));
    }
    let mut arguments = if matches!(args.action, Some(VerifyAction::Test)) {
        vec![
            "--test".into(),
            "--test-reporter=tap".into(),
            "--test-concurrency=1".into(),
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

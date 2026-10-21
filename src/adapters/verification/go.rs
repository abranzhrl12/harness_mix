use super::Project;
use harness_domain::{
    Failure,
    contracts::{VerifyAction, VerifyArgs},
    process::{ProcessSpec, Program},
};
use std::time::Duration;

pub(super) fn go_plan(project: &Project, args: &VerifyArgs) -> Result<ProcessSpec, Failure> {
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

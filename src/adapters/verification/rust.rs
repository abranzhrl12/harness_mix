use super::Project;
use harness_domain::{
    Failure,
    contracts::{VerifyAction, VerifyArgs},
    process::{ProcessSpec, Program},
};
use std::time::Duration;

pub(super) fn plan(project: &Project, args: &VerifyArgs) -> Result<ProcessSpec, Failure> {
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
        Some(VerifyAction::Build) => "build",
        None => "check",
    };
    let mut arguments = vec![
        action.into(),
        "--manifest-path".into(),
        manifest
            .to_str()
            .ok_or(("PATH", "Manifest path must be UTF-8"))?
            .into(),
        "--color".into(),
        "never".into(),
    ];
    if let Some(package) = &args.package {
        arguments.extend(["-p".into(), package.clone()]);
    }
    ProcessSpec::new(
        Program::Cargo,
        arguments,
        Duration::from_secs(60),
        2 * 1024 * 1024,
    )
}

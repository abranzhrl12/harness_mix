use super::Project;
use harness_domain::{
    Failure,
    contracts::{VerifyAction, VerifyArgs},
    process::{ProcessSpec, Program},
};
use std::time::Duration;

pub(super) fn python_plan(project: &Project, args: &VerifyArgs) -> Result<ProcessSpec, Failure> {
    let scope = args.scope.as_deref().ok_or((
        "VERIFY_SCOPE",
        "Python requires explicit file (build) or test directory (test)",
    ))?;
    if scope.len() > 4096 {
        return Err(("PATH", "Scope too long"));
    }
    let target = project.target(scope)?;
    let path = target
        .to_str()
        .ok_or(("PATH", "Python scope must be UTF-8"))?;
    let arguments = if matches!(args.action, Some(VerifyAction::Test)) {
        if !target.is_dir()
            && (!target.is_file() || target.extension().and_then(|e| e.to_str()) != Some("py"))
        {
            return Err((
                "VERIFY_SCOPE",
                "Python test requires a directory or .py test file",
            ));
        }
        let test_dir = if target.is_dir() {
            &target
        } else {
            target.parent().unwrap_or(&target)
        };
        let has_pytest = test_dir.join("pytest.ini").is_file()
            || test_dir.join("pyproject.toml").is_file()
            || test_dir.join("conftest.py").is_file()
            || project.root.join("pytest.ini").is_file()
            || project.root.join("pyproject.toml").is_file();

        if has_pytest {
            vec![
                "-B".into(),
                "-m".into(),
                "pytest".into(),
                path.into(),
                "-q".into(),
            ]
        } else if target.is_file() {
            vec![
                "-I".into(),
                "-S".into(),
                "-B".into(),
                "-m".into(),
                "unittest".into(),
                path.into(),
            ]
        } else {
            vec![
                "-I".into(),
                "-S".into(),
                "-B".into(),
                "-m".into(),
                "unittest".into(),
                "discover".into(),
                "-s".into(),
                path.into(),
                "-t".into(),
                path.into(),
                "-p".into(),
                "test*.py".into(),
            ]
        }
    } else {
        if !target.is_file() || target.extension().and_then(|e| e.to_str()) != Some("py") {
            return Err(("VERIFY_SCOPE", "Python build requires a .py file"));
        }
        vec![
            "-I".into(),
            "-S".into(),
            "-m".into(),
            "py_compile".into(),
            path.into(),
        ]
    };
    ProcessSpec::new(
        Program::Python,
        arguments,
        Duration::from_secs(30),
        2 * 1024 * 1024,
    )
}

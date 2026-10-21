use super::capture::run_command_observed;
use harness_domain::{
    Failure,
    execution::OperationContext,
    process::{ProcessSpec, Program, StopReason},
};
use std::{path::PathBuf, process::Command, time::Duration};

pub(in crate::adapters) fn executable(program: Program) -> Result<PathBuf, Failure> {
    if program == Program::Msbuild {
        return visual_studio_msbuild();
    }
    let name = match program {
        Program::Cargo => "cargo",
        Program::Rustc => "rustc",
        Program::Git => "git",
        Program::Go => "go",
        Program::Python => "python",
        Program::Node => "node",
        Program::Dotnet => "dotnet",
        Program::Msbuild => unreachable!(),
    };
    let filename = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    for directory in std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
    {
        // No current-directory or relative-PATH lookup, even on Windows.
        if !directory.is_absolute() {
            continue;
        }
        let path = directory.join(&filename);
        if !path.is_file() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if !path
                .metadata()
                .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
            {
                continue;
            }
        }
        return path
            .canonicalize()
            .map_err(|_| ("PROCESS_EXECUTABLE", "Executable unavailable"));
    }
    Err((
        "PROCESS_EXECUTABLE",
        "Required executable not found in absolute PATH entries",
    ))
}

#[cfg(any(windows, test))]
pub(super) fn has_visual_studio_web_targets(executable: &std::path::Path) -> bool {
    let Some(msbuild) = executable.ancestors().find(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("MSBuild"))
    }) else {
        return false;
    };
    let Ok(versions) = std::fs::read_dir(msbuild.join("Microsoft/VisualStudio")) else {
        return false;
    };

    versions.filter_map(Result::ok).take(128).any(|version| {
        version.file_name().to_string_lossy().starts_with('v')
            && version
                .path()
                .join("WebApplications/Microsoft.WebApplication.targets")
                .is_file()
    })
}

#[cfg(windows)]
fn visual_studio_msbuild() -> Result<PathBuf, Failure> {
    let installer = std::env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or((
            "MSBUILD_UNAVAILABLE",
            "Visual Studio Installer location unavailable",
        ))?
        .join("Microsoft Visual Studio/Installer/vswhere.exe");
    if !installer.is_file() {
        return Err((
            "MSBUILD_UNAVAILABLE",
            "Install Visual Studio Build Tools with Web build tools (vswhere.exe missing)",
        ));
    }
    let mut command = Command::new(installer);
    command.args([
        "-utf8",
        "-products",
        "*",
        "-requires",
        "Microsoft.Component.MSBuild",
        "-find",
        "MSBuild\\**\\Bin\\MSBuild.exe",
    ]);
    let spec = ProcessSpec::new(Program::Msbuild, vec![], Duration::from_secs(5), 16384)?;
    let outcome = run_command_observed(
        command,
        &spec,
        &OperationContext::with_timeout(Duration::from_secs(5)),
        &mut |_, _| {},
    )?;
    if outcome.reason != StopReason::Exited || outcome.exit_code != Some(0) {
        return Err((
            "MSBUILD_UNAVAILABLE",
            "vswhere failed to locate Visual Studio MSBuild",
        ));
    }
    let candidates: Vec<_> = String::from_utf8_lossy(&outcome.stdout)
        .lines()
        .map(|line| PathBuf::from(line.trim().trim_start_matches('\u{feff}')))
        .filter(|path| path.is_absolute() && path.is_file())
        .collect();
    // Prefer an installed WebApplication target, even when a newer VS instance
    // only has basic MSBuild. Explicit modern-project builds retain a fallback.
    candidates
        .iter()
        .find(|path| has_visual_studio_web_targets(path))
        .cloned()
        .or_else(|| candidates.into_iter().next())
        .ok_or((
            "MSBUILD_UNAVAILABLE",
            "Visual Studio MSBuild not found; install Visual Studio Web build tools",
        ))
}

#[cfg(not(windows))]
fn visual_studio_msbuild() -> Result<PathBuf, Failure> {
    Err((
        "MSBUILD_UNAVAILABLE",
        "Classic ASP.NET requires Visual Studio MSBuild on Windows",
    ))
}

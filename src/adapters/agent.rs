//! Native adapter for interactive, locally installed coding agents.
use harness_application::ports::agent::AgentLauncher;
use harness_domain::agent::AgentProvider;
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[derive(Clone, Copy, Debug, Default)]
pub struct AntigravityLauncher;

impl AntigravityLauncher {
    fn resolve(&self, provider: AgentProvider) -> Option<PathBuf> {
        if provider != AgentProvider::Antigravity {
            return None;
        }

        let executable_name = if cfg!(windows) { "agy.exe" } else { "agy" };
        if let Some(path) = env::var_os("PATH") {
            for directory in env::split_paths(&path).filter(|path| path.is_absolute()) {
                if let Some(path) = usable_executable(&directory.join(executable_name)) {
                    return Some(path);
                }
            }
        }

        #[cfg(windows)]
        if let Some(local_app_data) = env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            if local_app_data.is_absolute() {
                let fallback = local_app_data.join("agy").join("bin").join("agy.exe");
                if let Some(path) = usable_executable(&fallback) {
                    return Some(path);
                }
            }
        }

        None
    }
}

impl AgentLauncher for AntigravityLauncher {
    fn available(&self, provider: AgentProvider) -> bool {
        self.resolve(provider).is_some()
    }

    fn executable(&self, provider: AgentProvider) -> Option<PathBuf> {
        self.resolve(provider)
    }

    fn launch(&self, provider: AgentProvider, workspace: &Path) -> Result<i32, String> {
        let executable = self.resolve(provider).ok_or_else(missing_cli_message)?;
        let workspace = workspace
            .canonicalize()
            .map_err(|_| format!("Workspace is unavailable: {}", workspace.display()))?;
        if !workspace.is_dir() {
            return Err(format!(
                "Workspace must be a directory: {}",
                workspace.display()
            ));
        }

        let status = Command::new(&executable)
            .current_dir(&workspace)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|error| {
                format!(
                    "Could not start Antigravity CLI at {}: {error}",
                    executable.display()
                )
            })?;

        status
            .code()
            .ok_or_else(|| "Antigravity CLI exited without an exit code".to_owned())
    }
}

fn usable_executable(candidate: &Path) -> Option<PathBuf> {
    let canonical = candidate.canonicalize().ok()?;
    let metadata = canonical.metadata().ok()?;
    if !metadata.is_file() {
        return None;
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o111 == 0 {
        return None;
    }
    Some(canonical)
}

fn missing_cli_message() -> String {
    #[cfg(windows)]
    {
        "Antigravity CLI `agy.exe` was not found. Install Antigravity, add its executable to PATH, or check `%LOCALAPPDATA%\\agy\\bin\\agy.exe`.".to_owned()
    }
    #[cfg(not(windows))]
    {
        "Antigravity CLI `agy` was not found. Install Antigravity and add its executable to PATH."
            .to_owned()
    }
}

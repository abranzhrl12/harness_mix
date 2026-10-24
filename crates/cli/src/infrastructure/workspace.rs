use std::{env, path::PathBuf};

use crate::WorkspaceLocator;

/// Native workspace discovery formerly owned by the executable host.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeWorkspaceLocator;

impl WorkspaceLocator for NativeWorkspaceLocator {
    fn detect_workspace(&self) -> Result<PathBuf, String> {
        for var in [
            "WORKSPACE_ROOT",
            "VSCODE_CWD",
            "INIT_CWD",
            "PROJECT_ROOT",
            "ANTIGRAVITY_WORKSPACE",
            "GEMINI_WORKSPACE",
        ] {
            if let Some(val) = env::var_os(var) {
                let path = PathBuf::from(val);
                if path.is_dir() {
                    return Ok(path);
                }
            }
        }
        let cwd = env::current_dir().map_err(|_| "Cannot detect workspace root".to_owned())?;
        let mut curr = cwd.clone();
        loop {
            if curr.join(".git").exists() || curr.join(".agents").exists() {
                return Ok(curr);
            }
            if !curr.pop() {
                break;
            }
        }
        Ok(cwd)
    }

    fn current_dir(&self) -> Result<PathBuf, String> {
        env::current_dir().map_err(|error| error.to_string())
    }
}

//! Binary composition root for the reusable terminal CLI.
use agent_harness::adapters::acp::{
    AntigravityAcpAdapter, AntigravityAcpConfig, protocol_path,
};
use harness_cli::terminal::{ConversationConfig, ConversationFactory};
use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
};

struct AcpFactory;

impl ConversationFactory for AcpFactory {
    type Provider = AntigravityAcpAdapter;

    fn default_command(&self) -> Option<PathBuf> {
        env::var_os("HARNESSMIX_ANTIGRAVITY_ACP").map(PathBuf::from)
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn discover(&self, explicit: Option<&Path>) -> Option<PathBuf> {
        if let Some(path) = explicit {
            return executable_file(path);
        }
        let name = if cfg!(windows) {
            "agy_acp_server.exe"
        } else {
            "agy_acp_server.par"
        };
        if let Some(found) = from_path(name) {
            return Some(found);
        }
        if cfg!(windows) {
            env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .and_then(|path| {
                    executable_file(&path.join("harnessmix/agents/antigravity/1.4.3").join(name))
                })
        } else {
            None
        }
    }

    fn create(&self, config: ConversationConfig) -> Result<Self::Provider, String> {
        let workspace = protocol_path(&config.workspace);
        let executable = protocol_path(&config.executable);
        let mcp_servers = if config.harness_enabled {
            let harness = harness_executable().ok_or(
                "No se encontró agent-harness. Instálalo junto a harnessmix o usa --no-harness.",
            )?;
            let harness = protocol_path(&harness);
            vec![serde_json::json!({
                "name": "harness",
                "command": harness,
                "args": ["--root", workspace.to_string_lossy()],
                "env": []
            })]
        } else {
            Vec::new()
        };
        let helper_name = if cfg!(windows) {
            "localharness_external.exe"
        } else {
            "localharness_external"
        };
        let harness_helper = executable
            .parent()
            .map(|parent| parent.join(helper_name))
            .filter(|path| path.is_file());

        Ok(AntigravityAcpAdapter::new(AntigravityAcpConfig {
            executable,
            args: config.args,
            mcp_servers,
            harness_helper,
        }))
    }
}

fn executable_file(path: &Path) -> Option<PathBuf> {
    let path = path.canonicalize().ok()?;
    if !path.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if path.metadata().ok()?.permissions().mode() & 0o111 == 0 {
            return None;
        }
    }
    Some(path)
}

fn from_path(name: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|value| {
        env::split_paths(&value)
            .filter(|path| path.is_absolute())
            .find_map(|path| executable_file(&path.join(name)))
    })
}

fn harness_executable() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "agent-harness.exe"
    } else {
        "agent-harness"
    };
    env::current_exe()
        .ok()
        .and_then(|path| path.parent().and_then(|dir| executable_file(&dir.join(name))))
        .or_else(|| from_path(name))
}

#[tokio::main]
async fn main() -> Result<(), String> {
    harness_cli::terminal::run(std::env::args_os().skip(1).collect::<Vec<OsString>>(), &AcpFactory)
        .await
}

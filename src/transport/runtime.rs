mod coordinator;
mod jobs;
mod live_workspace;
mod single;
mod workspace;

pub use single::{serve, serve_with_logging};
pub use workspace::serve_with_workspace_config;

/// Resolves the D2 renderer and the minimal environment it may inherit. The MCP
/// crate never reads the process environment, so the native runtime does it.
fn d2_host() -> harness_mcp::D2Host {
    let executable = std::env::var_os("HARNESS_D2_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("d2"));
    let environment = ["PATH", "SYSTEMROOT", "TEMP", "TMP"]
        .into_iter()
        .filter(|name| cfg!(windows) || *name != "SYSTEMROOT")
        .filter_map(|name| std::env::var_os(name).map(|value| (name.to_owned(), value)))
        .collect();
    harness_mcp::D2Host { executable, environment }
}

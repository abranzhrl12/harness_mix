//! Bounded D2 rendering from the domain's safe diagram graph contract.
use harness_domain::diagram::{DiagramArgs, MAX_RENDER_BYTES};
use std::{ffi::OsString, path::PathBuf, time::Duration};

const RENDER_TIMEOUT: Duration = Duration::from_secs(120);

/// Where the D2 renderer lives and which variables it may see. The embedding
/// application resolves it from the process environment; this crate never does.
#[derive(Clone, Debug)]
pub struct D2Host {
    pub executable: PathBuf,
    pub environment: Vec<(String, OsString)>,
}

impl Default for D2Host {
    fn default() -> Self {
        Self { executable: PathBuf::from("d2"), environment: Vec::new() }
    }
}

pub(super) async fn render(args: DiagramArgs, host: &D2Host) -> Result<Vec<u8>, &'static str> {
    let source = args.to_d2()?;
    let temp = tempfile::Builder::new().prefix("harness-d2-").tempdir()
        .map_err(|_| "Could not create isolated temporary directory")?;
    let input = temp.path().join("diagram.d2");
    let output = temp.path().join("diagram.png");
    std::fs::write(&input, source.as_bytes()).map_err(|_| "Could not stage diagram source")?;

    let mut command = tokio::process::Command::new(&host.executable);
    command.arg(&input).arg(&output).current_dir(temp.path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .env_clear();
    for (name, value) in &host.environment {
        command.env(name, value);
    }

    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound { "D2 is unavailable; install D2 on the MCP server host and add it to PATH" }
        else { "Could not start the D2 renderer" }
    })?;
    let status = match tokio::time::timeout(RENDER_TIMEOUT, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(_)) => return Err("D2 renderer failed while running"),
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err("D2 rendering exceeded the 120 second time limit");
        }
    };
    if !status.success() { return Err("D2 could not render this diagram"); }
    let metadata = std::fs::metadata(&output).map_err(|_| "D2 produced no PNG output")?;
    if metadata.len() == 0 || metadata.len() > MAX_RENDER_BYTES { return Err("Rendered PNG is empty or exceeds the 1.25 MB limit"); }
    let bytes = std::fs::read(PathBuf::from(output)).map_err(|_| "Could not read rendered PNG")?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") { return Err("D2 output was not a valid PNG image"); }
    Ok(bytes)
}

#[cfg(test)]
#[path = "diagram_tests.rs"]
mod tests;

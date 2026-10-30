//! Bounded D2 rendering from the domain's safe diagram graph contract.
use harness_domain::diagram::{DiagramArgs, MAX_RENDER_BYTES};
use std::{path::PathBuf, time::Duration};

const RENDER_TIMEOUT: Duration = Duration::from_secs(120);

pub(super) async fn render(args: DiagramArgs) -> Result<Vec<u8>, &'static str> {
    let source = args.to_d2()?;
    let temp = tempfile::Builder::new().prefix("harness-d2-").tempdir()
        .map_err(|_| "Could not create isolated temporary directory")?;
    let input = temp.path().join("diagram.d2");
    let output = temp.path().join("diagram.png");
    std::fs::write(&input, source.as_bytes()).map_err(|_| "Could not stage diagram source")?;

    let executable = std::env::var_os("HARNESS_D2_PATH").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("d2"));
    let mut command = tokio::process::Command::new(executable);
    command.arg(&input).arg(&output).current_dir(temp.path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .env_clear();
    if let Some(path) = std::env::var_os("PATH") { command.env("PATH", path); }
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SYSTEMROOT") { command.env("SYSTEMROOT", root); }
    if let Some(temp_path) = std::env::var_os("TEMP") { command.env("TEMP", temp_path); }
    if let Some(temp_path) = std::env::var_os("TMP") { command.env("TMP", temp_path); }

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
mod tests {
    use super::*;
    use harness_domain::diagram::{DiagramEdge, DiagramFormat, DiagramNode};

    #[tokio::test]
    async fn renders_real_png_with_d2_when_binary_is_available() {
        let executable = std::env::var_os("HARNESS_D2_PATH").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("d2"));
        if tokio::process::Command::new(executable).arg("--version").output().await.is_err() {
            eprintln!("D2 rendering check skipped: d2 is not on PATH");
            return;
        }
        let args = DiagramArgs {
            nodes: vec![DiagramNode { id: "web".into(), label: "Web \"app\" \\ backend".into() }, DiagramNode { id: "db".into(), label: "Database".into() }],
            edges: vec![DiagramEdge { from: "web".into(), to: "db".into(), label: Some("queries".into()) }],
            format: DiagramFormat::Png,
        };
        let png = render(args).await.expect("D2 renders a PNG");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}










use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let (root, persist_logs) = match (args.next(), args.next(), args.next(), args.next()) {
        (Some(flag), Some(root), None, None) if flag == "--root" => (PathBuf::from(root), false),
        (Some(flag), Some(root), Some(option), None)
            if flag == "--root" && option == "--persist-logs" =>
        {
            (PathBuf::from(root), true)
        }
        (Some(flag), None, None, None) if flag == "--version" => {
            println!("agent-harness {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        _ => {
            return Err(
                "usage: agent-harness --root <workspace> [--persist-logs] | --version".into(),
            );
        }
    };
    agent_harness::transport::serve_with_logging(&root, persist_logs).await
}

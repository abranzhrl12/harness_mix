use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let root = match (args.next(), args.next(), args.next()) {
        (Some(flag), Some(root), None) if flag == "--root" => PathBuf::from(root),
        (Some(flag), None, None) if flag == "--version" => {
            println!("agent-harness {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        _ => return Err("usage: agent-harness --root <workspace> | --version".into()),
    };
    agent_harness::transport::serve(&root).await
}

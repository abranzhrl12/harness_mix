//! Isolated extraction bridge, not a production graph writer or MCP server.
use std::{env, fs, process};

const REVISION: &str = "cfd4d74132219f498d5ae611d7e94990a27c2c0d";

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args == ["--version"] {
        println!("rustcodegraph-probe {REVISION}");
        return Ok(());
    }
    let [logical_path, source_path] = args.as_slice() else {
        return Err("usage: harness-rustcodegraph-probe <logical-path> <fixture-file>".into());
    };
    if fs::metadata(source_path)?.len() > 1024 * 1024 {
        return Err("fixture source exceeds 1 MiB".into());
    }
    let source = fs::read_to_string(source_path)?;
    let result = rustcodegraph::extraction::tree_sitter::extract_from_source(
        logical_path,
        &source,
        None,
        None,
    );
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

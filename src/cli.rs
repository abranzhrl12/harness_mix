//! Minimal CLI entrypoint for the MCP server binary.

use std::path::PathBuf;

pub async fn run(
    args: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut iter = args.into_iter();
    let mut root: Option<PathBuf> = None;
    let mut workspace_config: Option<PathBuf> = None;
    let mut persist_logs = false;
    let mut reindex = false;

    while let Some(arg) = iter.next() {
        let s = arg.to_string_lossy();
        match s.as_ref() {
            "--version" | "-V" => {
                println!("agent-harness {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "serve" | "--stdio" | "--mcp" => {}
            "--root" => {
                let val = iter.next().ok_or("--root requires a path")?;
                root = Some(PathBuf::from(val));
            }
            "--workspace-config" | "--config" | "--workspace" => {
                let val = iter.next().ok_or("--workspace-config requires a path")?;
                let path = PathBuf::from(val);
                if path.is_dir() {
                    root = Some(path);
                } else {
                    workspace_config = Some(path);
                }
            }
            "--persist-logs" => {
                persist_logs = true;
            }
            "--reindex" => {
                reindex = true;
            }
            other if other.starts_with("--root=") => {
                root = Some(PathBuf::from(&other["--root=".len()..]));
            }
            other if other.starts_with("--workspace-config=") => {
                workspace_config = Some(PathBuf::from(&other["--workspace-config=".len()..]));
            }
            other if other.starts_with("--workspace=") => {
                let path = PathBuf::from(&other["--workspace=".len()..]);
                if path.is_dir() {
                    root = Some(path);
                } else {
                    workspace_config = Some(path);
                }
            }
            other => {
                return Err(format!("unknown argument: {other}").into());
            }
        }
    }

    if reindex {
        let target_root = match (root, workspace_config) {
            (Some(r), None) => r,
            (None, None) => std::env::current_dir()?,
            (Some(_), Some(_)) => {
                return Err("cannot specify both --root and --workspace-config".into());
            }
            (None, Some(_)) => {
                return Err("--reindex requires --root (or current directory)".into());
            }
        };
        let report = agent_harness::adapters::index_store::rebuild_workspace(
            &target_root,
            &harness_domain::execution::OperationContext::without_deadline(),
        )
        .map_err(|(code, message)| format!("{code}: {message}"))?;
        println!("{report}");
        return Ok(());
    }

    match (root, workspace_config) {
        (Some(root), None) => {
            agent_harness::transport::serve_with_logging(&root, persist_logs).await?;
        }
        (None, Some(config)) => {
            agent_harness::transport::serve_with_workspace_config(&config, persist_logs).await?;
        }
        (None, None) => {
            let cwd = std::env::current_dir()?;
            agent_harness::transport::serve_with_logging(&cwd, persist_logs).await?;
        }
        (Some(_), Some(_)) => {
            return Err("cannot specify both --root and --workspace-config".into());
        }
    }

    Ok(())
}

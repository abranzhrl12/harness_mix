use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let input = (args.next(), args.next(), args.next(), args.next());
    if args.next().is_some() {
        return Err("too many command-line arguments".into());
    }
    let (root, persist_logs) = match input {
        (Some(flag), Some(root), Some(option), None) if flag == "--root" && option == "--sync" => {
            let context = harness_domain::execution::OperationContext::default();
            let result = agent_harness::adapters::index_store::sync_workspace(
                &PathBuf::from(root),
                &context,
            )
            .map_err(|(code, message)| format!("{code}: {message}"))?;
            println!("{}", serde_json::to_string(&result)?);
            if result["selectedScopeApplied"] != true {
                return Err("Native synchronization incomplete; see JSON report for committed files and remaining work".into());
            }
            return Ok(());
        }
        (Some(flag), Some(root), Some(option), Some(relative))
            if flag == "--root" && option == "--sync-file" =>
        {
            let context = harness_domain::execution::OperationContext::default();
            let relative = relative.to_str().ok_or("source path must be UTF-8")?;
            let result = agent_harness::adapters::index_store::sync_file(
                &PathBuf::from(root),
                relative,
                &context,
            )
            .map_err(|(code, message)| format!("{code}: {message}"))?;
            println!("{}", serde_json::to_string(&result)?);
            return Ok(());
        }
        (Some(flag), Some(root), None, None) if flag == "--root" => (PathBuf::from(root), false),
        (Some(flag), Some(root), Some(option), None)
            if flag == "--root" && option == "--sync-plan" =>
        {
            let context = harness_domain::execution::OperationContext::default();
            let result =
                agent_harness::adapters::index_sync::plan_workspace(&PathBuf::from(root), &context)
                    .map_err(|(code, message)| format!("{code}: {message}"))?;
            println!("{}", serde_json::to_string(&result)?);
            return Ok(());
        }
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
                "usage: agent-harness --root <workspace> [--persist-logs | --sync-plan | --sync | --sync-file <relative>] | --version".into(),
            );
        }
    };
    agent_harness::transport::serve_with_logging(&root, persist_logs).await
}

use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let collected: Vec<_> = args.by_ref().collect();
    if let [root_flag, root, sync_flag, relative, config_flag, config] = collected.as_slice()
        && root_flag == "--root"
        && sync_flag == "--sync-file"
        && config_flag == "--tsconfig"
    {
        let context = harness_domain::execution::OperationContext::default();
        let result = agent_harness::adapters::index_store::sync_file_with_tsconfig(
            &PathBuf::from(root),
            relative.to_str().ok_or("source path must be UTF-8")?,
            config.to_str().ok_or("config path must be UTF-8")?,
            &context,
        )
        .map_err(|(code, message)| format!("{code}: {message}"))?;
        println!("{}", serde_json::to_string(&result)?);
        return Ok(());
    }
    let mut args = collected.into_iter();
    let input = (args.next(), args.next(), args.next(), args.next());
    if args.next().is_some() {
        return Err("too many command-line arguments".into());
    }
    let (root, persist_logs) = match input {
        (Some(flag), Some(root), Some(option), None)
            if flag == "--root" && option == "--reindex" =>
        {
            let context = harness_domain::execution::OperationContext::default();
            let result = agent_harness::adapters::index_store::rebuild_workspace(
                &PathBuf::from(root),
                &context,
            )
            .map_err(|(code, message)| format!("{code}: {message}"))?;
            println!("{}", serde_json::to_string(&result)?);
            if result["selectedScopeApplied"] != true {
                return Err("Native rebuild incomplete; see JSON report".into());
            }
            return Ok(());
        }
        (Some(flag), Some(root), Some(option), None)
            if flag == "--root" && option == "--init-index" =>
        {
            let context = harness_domain::execution::OperationContext::default();
            let result =
                agent_harness::adapters::index_store::initialize(&PathBuf::from(root), &context)
                    .map_err(|(code, message)| format!("{code}: {message}"))?;
            println!("{}", serde_json::to_string(&result)?);
            return Ok(());
        }
        (Some(flag), Some(root), Some(option), None) if flag == "--root" && option == "--sync" => {
            let context = harness_domain::execution::OperationContext::default();
            let result = agent_harness::adapters::index_store::sync_workspace_all(
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
                "usage: agent-harness --root <workspace> [--persist-logs | --init-index | --reindex | --sync-plan | --sync | --sync-file <relative> [--tsconfig <relative.json>]] | --version".into(),
            );
        }
    };
    agent_harness::transport::serve_with_logging(&root, persist_logs).await
}

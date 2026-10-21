use std::path::PathBuf;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod index_progress;

fn run_index(
    timeout_seconds: u64,
    run: impl FnOnce(
        &harness_domain::execution::OperationContext,
    ) -> Result<serde_json::Value, harness_domain::Failure>,
) -> Result<serde_json::Value, String> {
    let reporter = index_progress::IndexProgressReporter::new();
    let observer = reporter.clone();
    let context = harness_domain::execution::OperationContext::with_timeout(
        std::time::Duration::from_secs(timeout_seconds),
    )
    .with_progress(move |update| observer.report(update));
    let started = std::time::Instant::now();
    match run(&context) {
        Ok(mut report) => {
            report["durationMs"] = serde_json::json!(started.elapsed().as_millis());
            reporter.finish(if report["selectedScopeApplied"] == false {
                "partial"
            } else {
                "complete"
            });
            Ok(report)
        }
        Err((code, message)) => {
            reporter.finish("failed");
            Err(format!("{code}: {message}"))
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let mut collected: Vec<_> = args.by_ref().collect();
    if collected.len() == 1 && (collected[0] == "--help" || collected[0] == "-h") {
        println!(
            "Agent Harness — MCP por stdio\n\nUso: agent-harness [--root <proyecto>] [opción]\n\nSin --root: detecta el proyecto desde el entorno o directorio actual.\n\nOpciones:\n  --index <acción>       Mantenimiento unificado del índice: init, sync, rebuild, plan\n  --sync-file <ruta>     Sincronizar un archivo relativo al proyecto\n    --tsconfig <ruta>    Usar aliases de un tsconfig explícito\n  --persist-logs         Guardar logs del servidor\n  --version              Mostrar la versión\n  --help, -h             Mostrar esta ayuda\n\nAlias compatibles de índice: --init-index, --sync, --reindex, --sync-plan"
        );
        println!(
            "\nConfiguración opcional: --init-config crea .agents/harness.config.json sin sobrescribir. SQL funciona sin JSON usando una cadena en .env o variables de entorno."
        );
        return Ok(());
    }
    if collected.first().is_some_and(|flag| {
        [
            "--index",
            "--init-index",
            "--init-config",
            "--sync",
            "--reindex",
            "--sync-plan",
            "--sync-file",
            "--persist-logs",
        ]
        .iter()
        .any(|option| flag == *option)
    }) {
        let root = auto_detect_workspace()?;
        collected.splice(
            0..0,
            [std::ffi::OsString::from("--root"), root.into_os_string()],
        );
    }
    if let [flag, root, option] = collected.as_slice()
        && flag == "--root"
        && option == "--init-config"
    {
        let result = agent_harness::adapters::database_setup::initialize(&PathBuf::from(root))
            .map_err(|(code, message)| format!("{code}: {message}"))?;
        println!("{}", result);
        return Ok(());
    }
    if let [root_flag, root, sync_flag, relative, config_flag, config] = collected.as_slice()
        && root_flag == "--root"
        && sync_flag == "--sync-file"
        && config_flag == "--tsconfig"
    {
        let relative = relative.to_str().ok_or("source path must be UTF-8")?;
        let config = config.to_str().ok_or("config path must be UTF-8")?;
        let result = run_index(30, |context| {
            agent_harness::adapters::index_store::sync_file_with_tsconfig(
                &PathBuf::from(root),
                relative,
                config,
                context,
            )
        })?;
        println!("{}", serde_json::to_string(&result)?);
        return Ok(());
    }
    let mut args = collected.into_iter();
    let input = (
        args.next(),
        args.next(),
        args.next(),
        args.next(),
        args.next(),
    );
    if args.next().is_some() {
        return Err("too many command-line arguments".into());
    }
    let (root, persist_logs) = match input {
        (Some(flag), Some(root), Some(option), val, None)
            if flag == "--root"
                && (option == "--reindex"
                    || (option == "--index"
                        && val.as_deref() == Some(std::ffi::OsStr::new("rebuild")))) =>
        {
            // A bounded multi-file rebuild is not a single MCP request. Keep
            // cancellation/deadline checks, but allow the CLI batch five minutes.
            let result = run_index(300, |context| {
                agent_harness::adapters::index_store::rebuild_workspace(
                    &PathBuf::from(root),
                    context,
                )
            })?;
            println!("{}", serde_json::to_string(&result)?);
            if result["selectedScopeApplied"] != true {
                return Err("Native rebuild incomplete; see JSON report".into());
            }
            return Ok(());
        }
        (Some(flag), Some(root), Some(option), val, None)
            if flag == "--root"
                && (option == "--init-index"
                    || (option == "--index"
                        && val.as_deref() == Some(std::ffi::OsStr::new("init")))) =>
        {
            let result = run_index(30, |context| {
                agent_harness::adapters::index_store::initialize(&PathBuf::from(root), context)
            })?;
            println!("{}", serde_json::to_string(&result)?);
            return Ok(());
        }
        (Some(flag), Some(root), Some(option), val, None)
            if flag == "--root"
                && (option == "--sync"
                    || (option == "--index"
                        && val.as_deref() == Some(std::ffi::OsStr::new("sync")))) =>
        {
            // CLI batches need a workspace budget independent of MCP's 30s request deadline.
            let result = run_index(300, |context| {
                agent_harness::adapters::index_store::sync_workspace_all(
                    &PathBuf::from(root),
                    context,
                )
            })?;
            println!("{}", serde_json::to_string(&result)?);
            if result["selectedScopeApplied"] != true {
                return Err("Native synchronization incomplete; see JSON report for committed files and remaining work".into());
            }
            return Ok(());
        }
        (Some(flag), Some(root), Some(option), Some(relative), None)
            if flag == "--root" && option == "--sync-file" =>
        {
            let relative = relative.to_str().ok_or("source path must be UTF-8")?;
            let result = run_index(30, |context| {
                agent_harness::adapters::index_store::sync_file(
                    &PathBuf::from(root),
                    relative,
                    context,
                )
            })?;
            println!("{}", serde_json::to_string(&result)?);
            return Ok(());
        }
        (Some(flag), Some(root), Some(option), Some(relative), Some(force))
            if flag == "--root" && option == "--sync-file" && force == "--force" =>
        {
            let relative = relative.to_str().ok_or("source path must be UTF-8")?;
            let result = run_index(30, |context| {
                agent_harness::adapters::index_store::sync_file_forced(
                    &PathBuf::from(root),
                    relative,
                    context,
                )
            })?;
            println!("{}", serde_json::to_string(&result)?);
            return Ok(());
        }
        (Some(flag), Some(root), None, None, None) if flag == "--root" => {
            (PathBuf::from(root), false)
        }
        (Some(flag), Some(root), Some(option), val, None)
            if flag == "--root"
                && (option == "--sync-plan"
                    || (option == "--index"
                        && val.as_deref() == Some(std::ffi::OsStr::new("plan")))) =>
        {
            let result = run_index(30, |context| {
                agent_harness::adapters::index_sync::plan_workspace(&PathBuf::from(root), context)
            })?;
            println!("{}", serde_json::to_string(&result)?);
            return Ok(());
        }
        (Some(flag), Some(root), Some(option), None, None)
            if flag == "--root" && option == "--persist-logs" =>
        {
            (PathBuf::from(root), true)
        }
        (None, None, None, None, None) => {
            let root = auto_detect_workspace()
                .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            (root, false)
        }
        (Some(flag), None, None, None, None) if flag == "--version" => {
            println!("agent-harness {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        _ => {
            return Err(
                "usage: agent-harness [--root <workspace>] [--index <init|sync|rebuild|plan> | --persist-logs | --sync-file <relative> [--tsconfig <relative.json>]] | --version".into(),
            );
        }
    };
    agent_harness::transport::serve_with_logging(&root, persist_logs).await
}

fn auto_detect_workspace() -> Result<PathBuf, Box<dyn std::error::Error>> {
    // 1. Check IDE and CLI standard environment variables
    for var in [
        "WORKSPACE_ROOT",
        "VSCODE_CWD",
        "INIT_CWD",
        "PROJECT_ROOT",
        "ANTIGRAVITY_WORKSPACE",
        "GEMINI_WORKSPACE",
    ] {
        if let Some(val) = std::env::var_os(var) {
            let path = PathBuf::from(val);
            if path.is_dir() {
                return Ok(path);
            }
        }
    }
    // 2. Inspect current directory or walk up until finding a git or agents root
    if let Ok(cwd) = std::env::current_dir() {
        let mut curr = cwd.clone();
        loop {
            if curr.join(".git").exists() || curr.join(".agents").exists() {
                return Ok(curr);
            }
            if !curr.pop() {
                break;
            }
        }
        return Ok(cwd);
    }
    Err("Cannot detect workspace root".into())
}

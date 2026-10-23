use std::{ffi::OsString, path::PathBuf};

mod parser;
mod plan_summary;
mod skills_sync;
use parser::{Command, Root, needs_workspace_root, parse};

pub async fn run(
    args: impl IntoIterator<Item = OsString>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<_> = args.into_iter().collect();
    if needs_workspace_root(&args) && args.first().is_none_or(|arg| arg != "skills") {
        let root = auto_detect_workspace()?;
        args.splice(0..0, [OsString::from("--root"), root.into_os_string()]);
    }
    let command = parse(args).map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    match command {
        Command::Help => print_help(),
        Command::Version => println!("agent-harness {}", env!("CARGO_PKG_VERSION")),
        Command::InitConfig { root } => {
            let result = agent_harness::adapters::database_setup::initialize(&resolve_root(root)?)
                .map_err(|(code, message)| format!("{code}: {message}"))?;
            println!("{}", result);
        }
        Command::InitIndex { root } => {
            let root = resolve_root(root)?;
            let result = run_index(30, |context| {
                agent_harness::adapters::index_store::initialize(&root, context)
            })?;
            println!("{}", serde_json::to_string(&result)?);
        }
        Command::SyncWorkspace { root } => {
            let root = resolve_root(root)?;
            let result = run_index(300, |context| {
                agent_harness::adapters::index_store::sync_workspace_all(&root, context)
            })?;
            println!("{}", serde_json::to_string(&result)?);
            if result["selectedScopeApplied"] != true {
                return Err("Native synchronization incomplete; see JSON report for committed files and remaining work".into());
            }
        }
        Command::Rebuild { root } => {
            let root = resolve_root(root)?;
            let result = run_index(300, |context| {
                agent_harness::adapters::index_store::rebuild_workspace(&root, context)
            })?;
            println!("{}", serde_json::to_string(&result)?);
            if result["selectedScopeApplied"] != true {
                return Err("Native rebuild incomplete; see JSON report".into());
            }
        }
        Command::Plan { root, summary } => {
            let root = resolve_root(root)?;
            let result = run_index(30, |context| {
                agent_harness::adapters::index_sync::plan_workspace(&root, context)
            })?;
            if summary {
                let projected = plan_summary::summarize(&result)?;
                println!("{}", serde_json::to_string(&projected)?);
            } else {
                println!("{}", serde_json::to_string(&result)?);
            }
        }
        Command::SyncFile {
            root,
            relative,
            config,
            force,
        } => {
            let relative = relative.to_str().ok_or("source path must be UTF-8")?;
            let config = config
                .map(|config| {
                    config
                        .into_string()
                        .map_err(|_| "config path must be UTF-8")
                })
                .transpose()?;
            let root = resolve_root(root)?;
            let result = run_index(30, |context| match config {
                Some(config) => agent_harness::adapters::index_store::sync_file_with_tsconfig(
                    &root, relative, &config, context,
                ),
                None if force => {
                    agent_harness::adapters::index_store::sync_file_forced(&root, relative, context)
                }
                None => agent_harness::adapters::index_store::sync_file(&root, relative, context),
            })?;
            println!("{}", serde_json::to_string(&result)?);
        }
        Command::SkillsSync {
            root,
            check,
            dry_run,
            enable_auto,
            disable_auto,
        } => {
            let root = resolve_root(root)?;
            let report = skills_sync::run(
                &root,
                skills_sync::Options {
                    check,
                    dry_run,
                    enable_auto,
                    disable_auto,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if (check && (report.changes > 0 || report.conflicts > 0))
                || report.conflicts > 0
                || report.errors > 0
            {
                return Err(
                    "skill synchronization reported changes or conflicts; inspect the JSON report"
                        .into(),
                );
            }
        }
        Command::InitProject { root } => {
            let executable = std::env::current_exe()?;
            let report = skills_sync::initialize_project(&root, &executable)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if report.conflicts > 0 || report.errors > 0 {
                return Err(
                    "project initialization reported conflicts or errors; inspect the JSON report"
                        .into(),
                );
            }
        }
        Command::Serve {
            root,
            workspace_config,
            persist_logs,
        } => match workspace_config {
            Some(config) => {
                agent_harness::transport::serve_with_workspace_config(&config, persist_logs).await?
            }
            None => {
                let root = resolve_root(root)?;
                match skills_sync::sync_on_startup(&root) {
                    Ok(Some(report)) => {
                        if report.conflicts > 0 || report.errors > 0 {
                            eprintln!(
                                "harness skills auto-sync: {}",
                                serde_json::to_string(&report)?
                            );
                        }
                    }
                    Ok(None) => {}
                    Err(error) => eprintln!("harness skills auto-sync skipped: {error}"),
                }
                agent_harness::transport::serve_with_logging(&root, persist_logs).await?
            }
        },
    }
    Ok(())
}

fn resolve_root(root: Root) -> Result<PathBuf, Box<dyn std::error::Error>> {
    match root {
        Root::Explicit(root) => Ok(root),
        Root::Detect => auto_detect_workspace(),
        Root::DetectOrCurrent => Ok(auto_detect_workspace()
            .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))),
    }
}

fn run_index(
    timeout_seconds: u64,
    run: impl FnOnce(
        &harness_domain::execution::OperationContext,
    ) -> Result<serde_json::Value, harness_domain::Failure>,
) -> Result<serde_json::Value, String> {
    let reporter = crate::index_progress::IndexProgressReporter::new();
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
            let diagnostic =
                agent_harness::adapters::index_store::take_sync_diagnostic(code, message);
            match diagnostic {
                Some(diagnostic) => Err(format!(
                    "{code}: {message}; sqliteDiagnostic={}",
                    serde_json::to_string(&diagnostic).unwrap_or_else(|_| "{}".into())
                )),
                None => Err(format!("{code}: {message}")),
            }
        }
    }
}

fn print_help() {
    println!(
        "Agent Harness — MCP por stdio\n\nUso: agent-harness [--root <proyecto> | --workspace-config <manifest.json>] [opción]\n\nSin --root: detecta el proyecto desde el entorno o directorio actual.\n\nOpciones:\n  init --root <proyecto> Preparar skills, config local y ejemplo MCP del proyecto\n  --workspace-config <archivo> Servir varios proyectos con un manifiesto JSON\n  --index <acción>       Mantenimiento unificado del índice: init, sync, rebuild, plan\n  --sync-file <ruta>     Sincronizar un archivo relativo al proyecto\n    --tsconfig <ruta>    Usar aliases de un tsconfig explícito\n  skills sync            Instalar o actualizar skills Harness en el proyecto\n    --check              Comprobar diferencias, sin escribir\n    --dry-run            Mostrar el plan, sin escribir\n    --enable-auto        Activar sync local al arrancar este proyecto\n    --disable-auto       Desactivar sync automático local\n  --persist-logs         Guardar logs del servidor\n  --version              Mostrar la versión\n  --help, -h             Mostrar esta ayuda\n\nAlias compatibles de índice: --init-index, --sync, --reindex, --sync-plan"
    );
    println!(
        "\nConfiguración opcional: --init-config crea .agents/harness.config.json sin sobrescribir. SQL funciona sin JSON usando una cadena en .env o variables de entorno."
    );
    println!(
        "Resumen de plan: --index plan --summary o --sync-plan --summary. Reduce la respuesta JSON; el cálculo del plan no cambia."
    );
}

fn auto_detect_workspace() -> Result<PathBuf, Box<dyn std::error::Error>> {
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

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;

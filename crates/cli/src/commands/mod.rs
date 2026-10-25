//! Command execution and presentation.
//!
//! The CLI owns command policy and output. A host supplies the concrete
//! application operations through [`Host`], which keeps this crate reusable by
//! the binary and by tests without depending on the executable crate.

mod active;
mod plan_summary;
mod skills_sync;

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

use crate::{Command, index_progress::IndexProgressReporter, prepare_command, resolve_root};

/// Index operation selected by the CLI. Host adapters translate this small
/// enum to their concrete index implementation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexOperation {
    Initialize,
    SyncWorkspace,
    Rebuild,
    Plan,
    SyncFile {
        relative: String,
        tsconfig: Option<String>,
        force: bool,
    },
}

/// Database setup capability supplied by the executable or embedding app.
pub trait DatabaseHost {
    fn initialize_database(&self, root: &Path) -> Result<Value, String>;
}

/// Index maintenance capability supplied by the executable or embedding app.
pub trait IndexHost {
    fn index(
        &self,
        root: &Path,
        operation: IndexOperation,
        context: &OperationContext,
    ) -> Result<Value, Failure>;

    fn take_sync_diagnostic(&self, _code: &str, _message: &str) -> Option<Value> {
        None
    }
}

/// Agent discovery and launch capability supplied by the executable or app.
pub trait AgentHost {
    fn agent_executable(&self) -> Option<PathBuf>;
    fn launch_agent(&self, root: &Path, provider: &str) -> Result<i32, String>;
}

/// Server transport capability supplied by the executable or app.
pub trait ServerHost {
    // Callers await this locally, so the trait does not promise `Send`.
    fn serve(
        &self,
        root: Option<&Path>,
        workspace_config: Option<&Path>,
        persist_logs: bool,
    ) -> impl std::future::Future<Output = Result<(), String>>;
}

/// Locates the current executable for project initialization.
pub trait ExecutableHost {
    fn current_executable(&self) -> Result<PathBuf, String>;
}

/// Executable boundary composed from small capabilities.
pub trait Host: DatabaseHost + IndexHost + AgentHost + ServerHost + ExecutableHost {}

impl<T> Host for T where
    T: DatabaseHost + IndexHost + AgentHost + ServerHost + ExecutableHost + ?Sized
{
}

/// Parse and execute one CLI invocation using the supplied host services.
pub async fn run(
    args: impl IntoIterator<Item = OsString>,
    host: &impl Host,
    package_version: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let command =
        prepare_command(args).map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    match command {
        Command::Help => print_help(),
        Command::Version => println!("agent-harness {package_version}"),
        Command::Config => active::config(host.agent_executable().as_deref())?,
        Command::Active { root, provider } => {
            let root = resolve_root(root)?;
            active::run(&root, provider.as_deref(), host)?;
        }
        Command::InitConfig { root } => {
            println!("{}", host.initialize_database(&resolve_root(root)?)?);
        }
        Command::InitIndex { root } => {
            let result = run_index(host, IndexOperation::Initialize, &resolve_root(root)?, 30)?;
            println!("{}", serde_json::to_string(&result)?);
        }
        Command::SyncWorkspace { root } => {
            let result = run_index(
                host,
                IndexOperation::SyncWorkspace,
                &resolve_root(root)?,
                300,
            )?;
            println!("{}", serde_json::to_string(&result)?);
            if !workspace_sources_applied(&result) {
                return Err("Native synchronization incomplete; see JSON report for committed files and remaining work".into());
            }
        }
        Command::Rebuild { root } => {
            let result = run_index(host, IndexOperation::Rebuild, &resolve_root(root)?, 300)?;
            println!("{}", serde_json::to_string(&result)?);
            if !workspace_sources_applied(&result) {
                return Err("Native rebuild incomplete; see JSON report".into());
            }
        }
        Command::Plan { root, summary } => {
            let result = run_index(host, IndexOperation::Plan, &resolve_root(root)?, 30)?;
            let result = if summary {
                plan_summary::summarize(&result)?
            } else {
                result
            };
            println!("{}", serde_json::to_string(&result)?);
        }
        Command::SyncFile {
            root,
            relative,
            config,
            force,
        } => {
            let relative = relative
                .to_str()
                .ok_or("source path must be UTF-8")?
                .to_owned();
            let config = config
                .map(|config| {
                    config
                        .into_string()
                        .map_err(|_| "config path must be UTF-8")
                })
                .transpose()?;
            let result = run_index(
                host,
                IndexOperation::SyncFile {
                    relative,
                    tsconfig: config,
                    force,
                },
                &resolve_root(root)?,
                30,
            )?;
            println!("{}", serde_json::to_string(&result)?);
        }
        Command::SkillsSync {
            root,
            check,
            dry_run,
            enable_auto,
            disable_auto,
        } => {
            let report = skills_sync::run(
                &resolve_root(root)?,
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
            let executable = host.current_executable()?;
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
        } => {
            if let Some(config) = workspace_config.as_deref() {
                host.serve(None, Some(config), persist_logs).await?;
            } else {
                let root = resolve_root(root)?;
                match skills_sync::sync_on_startup(&root) {
                    Ok(Some(report)) if report.conflicts > 0 || report.errors > 0 => {
                        eprintln!(
                            "harness skills auto-sync: {}",
                            serde_json::to_string(&report)?
                        );
                    }
                    Ok(_) => {}
                    Err(error) => eprintln!("harness skills auto-sync skipped: {error}"),
                }
                host.serve(Some(&root), None, persist_logs).await?;
            }
        }
    }
    Ok(())
}

// Successful source publication may still have honest partial-graph warnings.
// Do not turn those warnings into a request to repeat the same rebuild.
fn workspace_sources_applied(report: &Value) -> bool {
    report["sourceScopeApplied"] == true || report["selectedScopeApplied"] == true
}

fn run_index(
    host: &impl IndexHost,
    operation: IndexOperation,
    root: &Path,
    timeout_seconds: u64,
) -> Result<Value, String> {
    let reporter = IndexProgressReporter::new();
    let observer = reporter.clone();
    let context = OperationContext::with_timeout(Duration::from_secs(timeout_seconds))
        .with_progress(move |update| observer.report(update));
    let started = Instant::now();
    match host.index(root, operation, &context) {
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
            match host.take_sync_diagnostic(&code, &message) {
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
        "Agentes: active abre el menú; active antigravity inicia Antigravity; config (o /config) muestra proveedores.\n"
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn source_publication_accepts_graph_warnings_but_not_unprocessed_files() {
        assert!(workspace_sources_applied(
            &serde_json::json!({"sourceScopeApplied":true,"selectedScopeApplied":false,"warningFiles":1})
        ));
        assert!(workspace_sources_applied(
            &serde_json::json!({"selectedScopeApplied":true})
        ));
        assert!(!workspace_sources_applied(
            &serde_json::json!({"sourceScopeApplied":false,"selectedScopeApplied":false,"failedFiles":1})
        ));
        assert!(!workspace_sources_applied(&serde_json::json!({})));
    }

    #[derive(Default)]
    struct FakeHost {
        index_call: Mutex<Option<IndexOperation>>,
        database_calls: Mutex<usize>,
        serve_calls: Mutex<usize>,
    }

    impl DatabaseHost for FakeHost {
        fn initialize_database(&self, _root: &Path) -> Result<Value, String> {
            *self.database_calls.lock().unwrap() += 1;
            Ok(serde_json::json!({"initialized": true}))
        }
    }

    impl IndexHost for FakeHost {
        fn index(
            &self,
            _root: &Path,
            operation: IndexOperation,
            _context: &OperationContext,
        ) -> Result<Value, Failure> {
            *self.index_call.lock().unwrap() = Some(operation);
            Ok(serde_json::json!({"selectedScopeApplied": true}))
        }
    }

    impl AgentHost for FakeHost {
        fn agent_executable(&self) -> Option<PathBuf> {
            None
        }
        fn launch_agent(&self, _root: &Path, _provider: &str) -> Result<i32, String> {
            Ok(0)
        }
    }

    impl ServerHost for FakeHost {
        async fn serve(
            &self,
            _root: Option<&Path>,
            _workspace_config: Option<&Path>,
            _persist_logs: bool,
        ) -> Result<(), String> {
            *self.serve_calls.lock().unwrap() += 1;
            Ok(())
        }
    }

    impl ExecutableHost for FakeHost {
        fn current_executable(&self) -> Result<PathBuf, String> {
            Ok(PathBuf::from("agent-harness"))
        }
    }

    #[tokio::test]
    async fn help_is_owned_by_cli_and_does_not_touch_host_capabilities() {
        let host = FakeHost::default();
        run([OsString::from("--help")], &host, "9.8.7")
            .await
            .unwrap();
        assert_eq!(*host.index_call.lock().unwrap(), None);
        assert_eq!(*host.database_calls.lock().unwrap(), 0);
        assert_eq!(*host.serve_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn parsed_index_command_is_translated_to_host_port() {
        let host = FakeHost::default();
        let args = ["--root", ".", "--init-index"]
            .into_iter()
            .map(OsString::from);
        run(args, &host, "9.8.7").await.unwrap();
        assert_eq!(
            *host.index_call.lock().unwrap(),
            Some(IndexOperation::Initialize)
        );
    }
}

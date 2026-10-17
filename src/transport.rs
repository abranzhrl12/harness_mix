use crate::adapters::{self, NativeApplication};
mod input;
use harness_application::{
    Application,
    ports::{SessionRepository, Workspace},
};
use harness_domain::contracts::{
    BootstrapArgs, DetailArgs, ReadArgs, Request, SearchArgs, VerifyArgs,
};
use harness_domain::execution::{DEFAULT_TIMEOUT, OperationContext};
use rmcp::{
    RoleServer, ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ServerCapabilities, ServerConfig},
    service::RequestContext,
    tool, tool_handler, tool_router,
};
use std::path::Path;

#[derive(Clone)]
struct McpHarness {
    application: NativeApplication,
}

impl McpHarness {
    async fn call(&self, request: Request, context: RequestContext<RoleServer>) -> CallToolResult {
        execute(&self.application, request, context.ct.cancelled()).await
    }
}

fn response(envelope: serde_json::Value) -> CallToolResult {
    if envelope["state"] == "ok" {
        CallToolResult::structured(envelope)
    } else {
        CallToolResult::structured_error(envelope)
    }
}

struct CancelOnDrop(OperationContext);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

async fn execute<W: Workspace + 'static, S: SessionRepository + 'static>(
    application: &Application<W, S>,
    request: Request,
    cancelled: impl std::future::Future<Output = ()>,
) -> CallToolResult {
    let name = request.name();
    let mutating = request.is_mutating();
    let args = request.arguments();
    let context = OperationContext::default();
    // Dropping a handler (client cancellation/disconnect) also signals its worker.
    let _cancel_on_drop = CancelOnDrop(context.clone());
    let operation = match application.prepare(request, context.clone()) {
        Ok(operation) => operation,
        Err(envelope) => return response(envelope),
    };
    let request_id = operation.request_id().to_owned();
    let mut worker = tokio::task::spawn_blocking(move || operation.run());
    let mut envelope = tokio::select! {
        result = &mut worker => match result { Ok(envelope) => envelope, Err(_) => application.failure(name, &args, ("INTERNAL", "operación no completada; si era edición, releer antes de reintentar")) },
        () = cancelled => {
            context.cancel();
            if mutating { worker.await.unwrap_or_else(|_| application.failure(name, &args, ("EDIT_OUTCOME_UNKNOWN", "Releer archivo antes de reintentar"))) }
            else { application.failure(name, &args, ("CANCELLED", "operación cancelada")) }
        },
        () = tokio::time::sleep(DEFAULT_TIMEOUT) => {
            context.cancel();
            if mutating { worker.await.unwrap_or_else(|_| application.failure(name, &args, ("EDIT_OUTCOME_UNKNOWN", "Releer archivo antes de reintentar"))) }
            else { application.failure(name, &args, ("TIMEOUT", "operación superó el plazo")) }
        },
    };
    // A detached worker retains its admission/session guards until it really stops.
    envelope["requestId"] = serde_json::json!(request_id);
    response(envelope)
}

#[tool_router]
impl McpHarness {
    #[tool(
        description = "Registrar/listar/revocar un alcance externo de solo lectura ligado a session durante 1 hora. grant requiere root absoluto y userAuthorized=true, solo si el usuario autorizó esa ruta para la tarea. No inferir autorización de archivos. No permite escritura, ejecución ni carpetas vecinas."
    )]
    async fn harness_external_scope(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::ExternalScopeArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::ExternalScope(args), context).await
    }
    #[tool(
        description = "Leer configuración XML/JSON redactada antes de truncar. XML: atributos/textos ocultos. JSON: sección top-level exacta; nombres de propiedades y escalares ocultos, claves duplicadas usan último valor. Sin referencias externas ni fuente original. Límite 1 MiB, 10000 nodos, profundidad 64. No modifica el runtime."
    )]
    async fn harness_config(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::ConfigArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Config(args), context).await
    }
    #[tool(
        description = "Logs nativos acotados, sin argumentos/rutas/contenido. Filtros limit 1..50, level, tool, slowOnly, requestId y after. Persistencia Windows opt-in con --persist-logs, histórico marcado; no lee logs Node."
    )]
    async fn harness_logs(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::LogsArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Logs(args), context).await
    }
    #[tool(
        description = "1–4 lecturas independientes de archivos, símbolos o Git. Máximo 2 hijos concurrentes; admisión y plazo compartidos. No mutaciones, verify, SQL ni batches anidados. Valida todos los tipos antes de iniciar; resultados por índice, no transacción."
    )]
    async fn harness_batch(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::BatchArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Batch(args), context).await
    }
    #[tool(
        description = "Diagnóstico nativo de solo lectura. versions=false por defecto solo localiza ejecutables; true ejecuta --version con límites. No builds ni reparaciones. CodeGraph/conexión y SQL aún no se diagnostican."
    )]
    async fn harness_doctor(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::DoctorArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Doctor(args), context).await
    }
    #[tool(
        description = "Estado Git de solo lectura, paginado y acotado. No fetch ni stage. Raíz Git o worktree vinculado registrado; excluye submódulos."
    )]
    async fn harness_git_status(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::GitStatusArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::GitStatus(args), context).await
    }
    #[tool(
        description = "Explorar relaciones de un símbolo exacto en un índice SQLite existente, sin Node. direction=inbound/outbound/both; maxDepth=1..4. Por defecto solo lectura. syncIndex=true autoriza actualizar el índice nativo ANTES de consultar (no crea índices); conserva el reporte de commits parciales y no consulta si sync queda incompleto. requireMatchingFiles=true exige hashes contribuyentes vigentes. No certifica cobertura global ni paridad con todos los resolvers del proveedor."
    )]
    async fn harness_explore(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::ExploreArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Explore(args), context).await
    }
    #[tool(
        description = "Diff Git textual de un archivo relativo, incluidas eliminaciones. Solo lectura; staged selecciona HEAD:index. Sin textconv/diff externo, archivos secretos ni metadatos Git. No incluye untracked."
    )]
    async fn harness_git_diff(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::GitDiffArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::GitDiff(args), context).await
    }
    #[tool(
        description = "Plan de sesión: status, update o timeline paginado. Fases declaradas por el agente; eventos observados separados. Persiste fases, no timeline. Una actualización iniciada espera su resultado de persistencia."
    )]
    async fn harness_tasks(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::TasksArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Tasks(args), context).await
    }

    #[tool(
        description = "Resumen de sesión y actualización de objetivo/decisiones/pendientes. refreshFreshness=true compara checks de la sesión viva con una huella local acotada. No certifica entorno, dependencias externas ni vigencia total; histórico no se revalida."
    )]
    async fn harness_checkpoint(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::CheckpointArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Checkpoint(args), context).await
    }
    #[tool(
        description = "Firmas Rust locales por AST nativo, sin Node ni CodeGraph. maxChars 1000..16000 limita JSON en unidades UTF-16; query prioriza nombres. Solo declaraciones y miembros de módulos inline/impl/trait; no relaciones."
    )]
    async fn harness_outline(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::OutlineArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Outline(args), context).await
    }

    #[tool(
        description = "Leer símbolo Rust por nombre, container o symbolId vigente. Rechaza ambigüedad. view=signature no sirve para editar; full devuelve fuente exacta y hash, con cuerpos grandes en detalle."
    )]
    async fn harness_read_symbol(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::ReadSymbolArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::ReadSymbol(args), context).await
    }
    #[tool(
        description = "Reemplazo único con SHA-256 vigente y validación previa. Windows: Rust, JSON, Markdown y texto plano. Sin creación de archivos. Rechaza rutas protegidas. Una publicación iniciada se completa aunque llegue cancelación; releer tras desconexión antes de reintentar."
    )]
    async fn harness_replace(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::ReplaceArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Replace(args), context).await
    }
    #[tool(
        description = "Verificar Rust (crate), Go (módulo), Python (.py build/directorio unittest), JS (.js/.cjs/.mjs check/test), dotnet/csharp (.csproj/.sln explícito con SDK, build/test --no-restore y servidores desactivados). No MSBuild clásico. Rutas locales validadas, sin scopes externos ni comandos/flags libres. Ejecuta código del proyecto, no es sandbox. Plazo 30 s/2 MiB."
    )]
    async fn harness_verify(
        &self,
        Parameters(args): Parameters<VerifyArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Verify(args), context).await
    }

    #[tool(
        description = "Diagnóstico y estado de la conexión a SQL Server de solo lectura."
    )]
    async fn harness_sql_status(
        &self,
        Parameters(args): Parameters<harness_domain::sql::SqlStatusArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::SqlStatus(args), context).await
    }

    #[tool(
        description = "Búsqueda en catálogo SQL Server (tablas, vistas, columnas, procedimientos, funciones)."
    )]
    async fn harness_sql_search(
        &self,
        Parameters(args): Parameters<harness_domain::sql::SqlSearchArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::SqlSearch(args), context).await
    }

    #[tool(
        description = "Describe columnas, tipos, definiciones o relaciones de objetos SQL Server."
    )]
    async fn harness_sql_describe(
        &self,
        Parameters(args): Parameters<harness_domain::sql::SqlDescribeArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::SqlDescribe(args), context).await
    }

    #[tool(
        description = "Ejecuta una consulta SELECT validada de solo lectura en SQL Server con límite de filas y caracteres."
    )]
    async fn harness_sql_query(
        &self,
        Parameters(args): Parameters<harness_domain::sql::SqlQueryArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::SqlQuery(args), context).await
    }

    #[tool(
        description = "Bootstrap experimental Rust: reglas, capacidades y sesiones persistentes. No reemplaza todavía el harness Node."
    )]
    async fn harness_bootstrap(
        &self,
        Parameters(args): Parameters<BootstrapArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Bootstrap(args), context).await
    }

    #[tool(
        description = "Leer hasta 100 líneas UTF-8 dentro del workspace con hash SHA-256 y hasta dos líneas de contexto por lado."
    )]
    async fn harness_read(
        &self,
        Parameters(args): Parameters<ReadArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Read(args), context).await
    }

    #[tool(
        description = "Recuperar detalle por referencia local al proceso, con paginación UTF-16 compatible con JavaScript. Máximo 16000 caracteres."
    )]
    async fn harness_detail(
        &self,
        Parameters(args): Parameters<DetailArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Detail(args), context).await
    }

    #[tool(
        description = "Búsqueda nativa text/files/file_size/syntax. syntax: solo declaraciones Rust, selector declaration_kind[:nombre exacto], p.ej. function_item:run o struct_item; no patrones AST generales ni callers. file_size no recibe query: minLines=500 filtra archivos con más líneas. Límites 2 MiB/archivo, 5 s cooperativos, salida parcial explícita. Admite scopes externos autorizados Windows."
    )]
    async fn harness_search(
        &self,
        Parameters(args): Parameters<SearchArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Search(args), context).await
    }

    #[tool(
        description = "Inspecciona metadatos y hojas de un libro XLSX de solo lectura sin volcar filas (hash SHA-256, hojas, dimensiones)."
    )]
    async fn harness_xlsx_inspect(
        &self,
        Parameters(args): Parameters<harness_domain::xlsx::XlsxInspectArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::XlsxInspect(args), context).await
    }

    #[tool(
        description = "Lee un rango paginado y acotado de celdas de una hoja XLSX en formato matricial/JSON."
    )]
    async fn harness_xlsx_read(
        &self,
        Parameters(args): Parameters<harness_domain::xlsx::XlsxReadArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::XlsxRead(args), context).await
    }

    #[tool(
        description = "Inspecciona o previsualiza SVG estático seguro de hasta 256 KiB. Modo inspect: árbol acotado, dimensiones y referencias. Modo preview: metadatos de renderizado seguro para modelos de visión."
    )]
    async fn harness_svg(
        &self,
        Parameters(args): Parameters<harness_domain::svg::SvgArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Svg(args), context).await
    }

    #[tool(
        description = "Genera un reporte Markdown de traspaso de sesión (summary, adr o pr_body). Si targetFile se especifica, publica atómicamente el archivo (.md); de lo contrario devuelve un preview acotado con el texto completo en detalle."
    )]
    async fn harness_handover(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::HandoverArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Handover(args), context).await
    }

    #[tool(
        description = "Consulta estado de procesos (existencia por pid, hijos retenidos), sondeo de puerto TCP y healthcheck HTTP(S) loopback."
    )]
    async fn harness_process_status(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::ProcessStatusArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::ProcessStatus(args), context).await
    }

    #[tool(
        description = "Diagnóstico rápido de sintaxis en lote de archivos explícitos o cambios Git tracked. Valida Rust, JS/TS y JSON sin devolver código. maxFiles 1–50 (defecto 20)."
    )]
    async fn harness_diagnostics(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::DiagnosticsArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Diagnostics(args), context).await
    }

    #[tool(
        description = "Consulta, filtra y proyecta datos estructurados JSON o archivos de configuracion en memoria con JMESPath sin procesos externos. maxChars 500-32000."
    )]
    async fn harness_data_query(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::DataQueryArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::DataQuery(args), context).await
    }
}

#[tool_handler]
impl ServerHandler for McpHarness {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("Experimental Rust harness with bounded reads/search, Git, workflow and diagnostics. Migration incomplete; do not replace production configuration yet.")
    }
}

pub async fn serve(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    serve_with_logging(root, false).await
}
pub async fn serve_with_logging(
    root: &Path,
    persist_logs: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let service = McpHarness {
        application: adapters::application_with_logging(root, persist_logs)?,
    }
    .serve((
        input::BoundedLines::new(tokio::io::stdin(), input::MAX_MESSAGE_BYTES),
        tokio::io::stdout(),
    ))
    .await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_application::ports::SessionContext;
    use harness_domain::Failure;
    use serde_json::{Value, json};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;
    use tokio::sync::oneshot;

    struct WaitingWorkspace {
        started: Mutex<Option<oneshot::Sender<()>>>,
        finished: Mutex<Option<oneshot::Sender<()>>>,
    }
    impl Workspace for WaitingWorkspace {
        fn replace(
            &self,
            _: &harness_domain::contracts::ReplaceArgs,
            context: &OperationContext,
        ) -> Result<Value, Failure> {
            self.started
                .lock()
                .unwrap()
                .take()
                .unwrap()
                .send(())
                .unwrap();
            // Simulate cancellation after the publication has begun.
            while context.check().is_ok() {
                std::thread::park_timeout(Duration::from_millis(1));
            }
            self.finished
                .lock()
                .unwrap()
                .take()
                .unwrap()
                .send(())
                .unwrap();
            Ok(json!({"changed":true,"commitStatus":"committed"}))
        }
        fn info(&self) -> Value {
            json!({})
        }
        fn rules(&self, _: &OperationContext) -> Result<String, Failure> {
            Ok(String::new())
        }
        fn capabilities(&self) -> Value {
            json!({})
        }
        fn read(&self, _: &ReadArgs, context: &OperationContext) -> Result<Value, Failure> {
            self.started
                .lock()
                .unwrap()
                .take()
                .unwrap()
                .send(())
                .unwrap();
            while context.check().is_ok() {
                std::thread::park_timeout(Duration::from_millis(1));
            }
            self.finished
                .lock()
                .unwrap()
                .take()
                .unwrap()
                .send(())
                .unwrap();
            context.check()?;
            unreachable!()
        }
        fn search(&self, _: &SearchArgs, _: &OperationContext) -> Result<Value, Failure> {
            unreachable!()
        }
    }
    struct TrackingSessions(Arc<AtomicUsize>, Arc<Mutex<()>>);
    impl SessionRepository for TrackingSessions {
        fn workflow_gate(&mut self, _: &str) -> Result<Arc<Mutex<()>>, Failure> {
            Ok(self.1.clone())
        }
        fn acquire(&mut self, _: &str) -> Result<(), Failure> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn release(&mut self, _: &str) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
        fn open(&mut self, _: Option<&str>) -> Result<SessionContext, Failure> {
            unreachable!()
        }
        fn update_hashes(&mut self, _: &str, _: String, _: String) -> Result<(), Failure> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn mutation_cancellation_waits_for_definitive_commit_outcome() {
        let (started_tx, started_rx) = oneshot::channel();
        let (finished_tx, finished_rx) = oneshot::channel();
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let active = Arc::new(AtomicUsize::new(0));
        let application = Application::new(
            WaitingWorkspace {
                started: Mutex::new(Some(started_tx)),
                finished: Mutex::new(Some(finished_tx)),
            },
            TrackingSessions(active.clone(), Arc::default()),
        );
        let request = Request::Replace(harness_domain::contracts::ReplaceArgs {
            path: "file.txt".into(),
            hash: "unused-by-fake".into(),
            old_text: "old".into(),
            new_text: "new".into(),
            session: Some("session".into()),
        });
        let task = tokio::spawn(async move {
            execute(&application, request, async {
                let _ = cancel_rx.await;
            })
            .await
        });
        tokio::time::timeout(Duration::from_secs(2), started_rx)
            .await
            .unwrap()
            .unwrap();
        cancel_tx.send(()).unwrap();
        let response = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        finished_rx.await.unwrap();
        assert_eq!(response.structured_content.as_ref().unwrap()["state"], "ok");
        assert_eq!(
            response.structured_content.as_ref().unwrap()["data"]["changed"],
            true
        );
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cancellation_and_handler_drop_stop_workers_and_release_session_leases() {
        for abort_handler in [false, true] {
            let (started_tx, started_rx) = oneshot::channel();
            let (finished_tx, finished_rx) = oneshot::channel();
            let (cancel_tx, cancel_rx) = oneshot::channel();
            let active = Arc::new(AtomicUsize::new(0));
            let application = Application::new(
                WaitingWorkspace {
                    started: Mutex::new(Some(started_tx)),
                    finished: Mutex::new(Some(finished_tx)),
                },
                TrackingSessions(active.clone(), Arc::default()),
            );
            let recovered = application.clone();
            let request = Request::Read(ReadArgs {
                external_scope: None,
                path: "waiting".into(),
                start: 1,
                end: 1,
                session: Some("session".into()),
            });
            let handler = tokio::spawn(async move {
                execute(&application, request, async {
                    let _ = cancel_rx.await;
                })
                .await
            });
            tokio::time::timeout(Duration::from_secs(3), started_rx)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(active.load(Ordering::SeqCst), 1);
            if abort_handler {
                handler.abort();
                let _ = handler.await;
            } else {
                cancel_tx.send(()).unwrap();
                let result = tokio::time::timeout(Duration::from_secs(3), handler)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(result.structured_content.unwrap()["state"], "cancelled");
            }
            tokio::time::timeout(Duration::from_secs(3), finished_rx)
                .await
                .unwrap()
                .unwrap();
            tokio::time::timeout(Duration::from_secs(3), async {
                while active.load(Ordering::SeqCst) != 0 {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .unwrap();
            // All four admissions are available again after physical worker completion.
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    let jobs: Result<Vec<_>, _> = (0..4)
                        .map(|_| {
                            recovered.prepare(
                                Request::Detail(DetailArgs {
                                    reference: "unused".into(),
                                    offset: None,
                                    max_chars: None,
                                    session: None,
                                }),
                                OperationContext::default(),
                            )
                        })
                        .collect();
                    if let Ok(jobs) = jobs {
                        drop(jobs);
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("leaked admission");
        }
    }
}

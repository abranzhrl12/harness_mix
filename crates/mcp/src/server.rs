//! MCP routing; runtime transport and application composition are supplied by callers.
use harness_application::{
    Application,
    ports::{JobService, SessionRepository, Workspace},
};
#[path = "execution.rs"]
mod execution;
#[path = "input.rs"]
pub(crate) mod input;
#[path = "jobs.rs"]
mod jobs;
#[path = "presentation.rs"]
mod presentation;
#[path = "profile.rs"]
mod profile;
#[path = "wire.rs"]
mod wire;
use execution::execute;
use harness_domain::contracts::{
    BootstrapArgs, BrowserArgs, DetailArgs, ReadArgs, Request, SearchArgs, VerifyArgs,
};
use presentation::response_with_options as present_response;
#[cfg(test)]
use presentation::{response_with_compact, response_with_presentation};
use rmcp::{
    RoleServer, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ServerCapabilities, ServerConfig},
    service::RequestContext,
    tool, tool_handler, tool_router,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub struct McpServer<W, S> {
    application: Application<W, S>,
    application_source: Option<Arc<dyn Fn() -> Application<W, S> + Send + Sync>>,
    tasks: Option<Arc<dyn JobService>>,
    task_notifications: Arc<AtomicBool>,
    task_forwarding: Arc<AtomicBool>,
    eager_tools: bool,
    context_presentation: bool,
}

impl<W, S> Clone for McpServer<W, S> {
    fn clone(&self) -> Self {
        Self {
            application: self.application.clone(),
            application_source: self.application_source.clone(),
            tasks: self.tasks.clone(),
            task_notifications: self.task_notifications.clone(),
            task_forwarding: self.task_forwarding.clone(),
            eager_tools: self.eager_tools,
            context_presentation: self.context_presentation,
        }
    }
}

impl<W: Workspace + 'static, S: SessionRepository + 'static> McpServer<W, S> {
    /// Creates an MCP service for the supplied application and optional job service.
    pub fn new(application: Application<W, S>, tasks: Option<Arc<dyn JobService>>) -> Self {
        Self {
            application,
            application_source: None,
            tasks,
            task_notifications: Arc::new(AtomicBool::new(true)),
            task_forwarding: Arc::new(AtomicBool::new(false)),
            eager_tools: true,
            context_presentation: false,
        }
    }

    /// Supplies a fresh routing snapshot before each operational request.
    pub fn with_application_source(
        mut self,
        source: impl Fn() -> Application<W, S> + Send + Sync + 'static,
    ) -> Self {
        self.application_source = Some(Arc::new(source));
        self
    }

    /// Enables eager-load metadata for always-load tools (default: `true`).
    pub fn with_eager_tools(mut self, eager_tools: bool) -> Self {
        self.eager_tools = eager_tools;
        self
    }

    /// Enables context-style exploration output (default: `false`).
    pub fn with_context_presentation(mut self, context_presentation: bool) -> Self {
        self.context_presentation = context_presentation;
        self
    }

    fn profile_router(&self) -> rmcp::handler::server::router::tool::ToolRouter<Self> {
        profile::router(
            Self::tool_router(),
            self.application.tool_profile(),
            self.eager_tools,
        )
    }
    async fn call(&self, request: Request, context: RequestContext<RoleServer>) -> CallToolResult {
        let application = match &self.application_source {
            Some(source) => {
                let source = source.clone();
                match tokio::task::spawn_blocking(move || source()).await {
                    Ok(application) => application,
                    Err(_) => {
                        return present_response(
                            harness_domain::results::Results::default().envelope(
                                request.name(),
                                &request.arguments(),
                                Err(("INTERNAL", "Workspace snapshot provider failed")),
                            ),
                            false,
                            self.context_presentation,
                        );
                    }
                }
            }
            None => self.application.clone(),
        };
        execute(
            &application,
            request,
            context.ct.cancelled(),
            self.context_presentation,
        )
        .await
    }
}

#[tool_router]
impl<W, S> McpServer<W, S>
where
    W: Workspace + 'static,
    S: SessionRepository + 'static,
{
    #[tool(
        description = "Verificación en segundo plano: action=start|status|list|events|observe|cancel. Ejemplos: {\"action\":\"start\",\"verify\":{\"language\":\"rust\",\"action\":\"test\"}}, {\"action\":\"list\"}, {\"action\":\"observe\",\"taskId\":\"<id devuelto>\"}. status, observe y cancel requieren taskId; start requiere verify."
    )]
    async fn harness_jobs(
        &self,
        Parameters(args): Parameters<harness_domain::tasks::TaskArgs>,
    ) -> CallToolResult {
        jobs::handle(self.tasks.as_deref(), args, self.context_presentation).await
    }

    #[tool(description = "Scope externo de solo lectura: action=grant|list|revoke.")]
    async fn harness_external_scope(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::ExternalScopeArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::ExternalScope(args), context).await
    }
    #[tool(description = "Lee configuración XML/JSON redactada por sección.")]
    async fn harness_config(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::ConfigArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Config(args), context).await
    }
    #[tool(
        description = "Conecta o inspecciona un navegador Chromium: action=connect|targets|attach|inspect|errors|screenshot|disconnect. connect acepta http://localhost:9222 (descubre /json/version) o WebSocket CDP directo ws://loopback:puerto/devtools/browser/.... Ejemplo: {\"action\":\"connect\",\"endpoint\":\"http://localhost:9222\",\"session\":\"<data.session>\"}. El navegador debe estar ejecutándose con depuración remota; solo loopback, sin redirects. Devuelve errores de consola/red, DOM y medidas del elemento seleccionado."
    )]
    async fn harness_browser(
        &self,
        Parameters(args): Parameters<BrowserArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Browser(args), context).await
    }
    #[tool(
        description = "Ejecuta 1–4 lecturas independientes en paralelo; admite harness_explore con syncIndex=false."
    )]
    async fn harness_batch(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::BatchArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Batch(args), context).await
    }
    #[tool(description = "Diagnóstico: action=env|syntax|logs|process|health.")]
    async fn harness_doctor(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::DoctorToolArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let is_env = matches!(&args, harness_domain::contracts::DoctorToolArgs::Env(_));
        let request = args.into_request();
        let mut result = self.call(request, context).await;
        if is_env {
            if let Some(mut envelope) = result.structured_content.take() {
                if envelope["state"] == "ok" {
                    let mut actual: Vec<String> = self
                        .profile_router()
                        .map
                        .keys()
                        .map(|name| name.to_string())
                        .collect();
                    let mut expected = self.application.tool_profile().tools();
                    actual.sort();
                    expected.sort();
                    let matches = actual == expected;
                    envelope["data"]["mcpRegistry"] = serde_json::json!({"status":if matches {"verified"} else {"error"},"profile":self.application.tool_profile().name(),"expected":expected,"advertised":actual});
                    harness_domain::diagnostics::append_check(
                        &mut envelope["data"],
                        serde_json::json!({"id":"mcp-registry","status":if matches {"ok"} else {"error"},"message":"Actual advertised MCP route names compared with the profile contract; function behavior is not certified"}),
                    );
                }
                return present_response(envelope, false, self.context_presentation);
            }
        }
        result
    }
    #[tool(
        description = "Git de solo lectura: action=status|diff. diff tomará path=. (workspace completo) si lo omites; usa path para un archivo o carpeta."
    )]
    async fn harness_git(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::GitArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(args.into_request(), context).await
    }
    #[tool(
        description = "Navega llamadas y dependencias sin encadenar búsquedas. Usa filePath + symbol directamente si los conoces. view=summary: relaciones y firmas; full (por defecto): también cuerpos; impact: impacto entrante. No sincroniza el índice por defecto. structuredContent contiene el sobre canónico con data, frescura, límites y detalle; content conserva el JSON textual compatible. Ejemplo: {\"filePath\":\"src/main.rs\",\"symbol\":\"main\",\"view\":\"summary\"}. Requiere un índice compatible; no lo crea."
    )]
    async fn harness_explore(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::ExploreArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Explore(args), context).await
    }
    #[tool(description = "Sesión: action=checkpoint|tasks|status|handover.")]
    async fn harness_session(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::SessionArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(args.into_request(), context).await
    }
    #[tool(
        description = "Símbolos locales con presupuesto de lectura de 16 MiB; la navegación JS/TS admite ese tamaño y otros analizadores conservan sus límites propios. Exceder el presupuesto de lectura devuelve SIZE sin análisis parcial. outline + path lista firmas; read + path + symbol devuelve fuente exacta. No requiere view ni límites. structuredContent contiene el sobre canónico; content conserva el JSON textual compatible. Conserva avisos y detalle si hay omisiones. Ejemplos: {\"action\":\"outline\",\"path\":\"src/main.rs\"}, {\"action\":\"read\",\"path\":\"src/main.rs\",\"symbol\":\"main\"}. HTML: symbol=html:#id; CSS: nombre del selector."
    )]
    async fn harness_symbols(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::SymbolsArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(args.into_request(), context).await
    }
    #[tool(
        description = "Reemplaza oldText único o por line; acepta hash=matches[].hash y oldText=matches[].context.text de harness_search cuando el contexto sea suficiente y siga vigente. hash es opcional: si se omite, se usa la lectura interna. La sustitución requiere oldText único salvo que line lo desambigüe; preview=true no publica. Acepta snake_case. HTML (.html/.htm) nativo sin allowUnvalidatedText; no valida sintaxis. Para plantillas Angular verifica después con harness_verify language=angular."
    )]
    async fn harness_replace(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::ReplaceArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Replace(args), context).await
    }
    #[tool(
        description = "Crea un archivo UTF-8; expectedHash reemplaza si coincide. HTML (.html/.htm) nativo sin allowUnvalidatedText; no valida sintaxis. Para plantillas Angular verifica después con harness_verify language=angular."
    )]
    async fn harness_write(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::WriteArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Write(args), context).await
    }
    #[tool(
        description = "Prepara una propuesta persistente: action=prepare, path y content completo. Guarda original y candidato, devuelve proposalId, hashes y diff sin modificar el archivo. expectedHash opcional; no acepta ni rechaza propuestas todavía. Escribe solo metadatos locales; requiere autorización para preparar."
    )]
    async fn harness_proposal(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::ProposalArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Proposal(args), context).await
    }
    #[tool(description = "Mueve o renombra un archivo dentro del workspace.")]
    async fn harness_move(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::MoveArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Move(args), context).await
    }
    #[tool(
        description = "Elimina permanentemente un archivo regular del workspace cuando el usuario lo autoriza. path relativo; expectedHash opcional impide borrar una versión distinta. No admite directorios ni enlaces. Comprueba changed, verified e index.status antes de reintentar."
    )]
    async fn harness_delete(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::DeleteArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Delete(args), context).await
    }
    #[tool(
        description = "Compila o prueba con herramientas locales, sin shell. .NET: language=dotnet, scope obligatorio a un archivo .csproj o .sln del workspace, engine=auto|dotnet|msbuild, action=build|test. Requiere SDK y restore previo; usa --no-restore; MSBuild solo admite build. Ejemplo: {\"language\":\"dotnet\",\"scope\":\"src/App/App.csproj\",\"action\":\"build\",\"engine\":\"auto\"}. Rust: {\"language\":\"rust\",\"action\":\"test\",\"package\":\"nombre-del-paquete\"}. language=angular invoca @angular/compiler-cli local sin emitir archivos; project/configuration seleccionan angular.json. timeoutSeconds: 1..3600, predeterminado 900."
    )]
    async fn harness_verify(
        &self,
        Parameters(args): Parameters<VerifyArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Verify(args), context).await
    }

    #[tool(
        description = "SQL de solo lectura: action=map|query|status|search|describe. Campos OBLIGATORIOS por acción: describe -> {\"action\":\"describe\",\"name\":\"dbo.Objeto\"} (kind/objectType es OPCIONAL y se autodetecta; acepta alias object/table/procedure/routine para name); search -> {\"action\":\"search\",\"query\":\"texto\"} (objectType opcional); query -> {\"sql\":\"SELECT ...\"} (action opcional si pasas sql); map -> {\"action\":\"map\"}; status -> {\"action\":\"status\"}. Solo SELECT/WITH; no ejecuta DDL ni escrituras."
    )]
    async fn harness_sql(
        &self,
        Parameters(args): Parameters<harness_domain::sql::SqlArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(args.into_request(), context).await
    }

    #[tool(description = "Inicia o reutiliza la sesión; compact=true reduce el catálogo.")]
    async fn harness_bootstrap(
        &self,
        Parameters(args): Parameters<BootstrapArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Bootstrap(args), context).await
    }

    #[tool(
        description = "Lee hasta 420 líneas UTF-8 de archivos de hasta 2 MiB, con hash y contexto; start/end son inclusivos, base 1. Ejemplo: {\"path\":\"src/main.rs\",\"start\":1,\"end\":80}. Alternativa: lines en lugar de end; no combinarlos. Por defecto lee desde start=1 hasta 120 líneas. compact=true omite metadatos vacíos del sobre; lineNumbers=true añade data.numberedText de visualización sin alterar data.text."
    )]
    async fn harness_read(
        &self,
        Parameters(args): Parameters<ReadArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Read(args), context).await
    }

    #[tool(description = "Recupera detalle paginado por ref (máx. 16000 chars).")]
    async fn harness_detail(
        &self,
        Parameters(args): Parameters<DetailArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Detail(args), context).await
    }

    #[tool(
        description = "Busca text/files/file_size/syntax con scope y globs. `limit` devuelve una página; si hay más filas, sigue el `nextCall` con `cursor` y `offset` para recorrer el snapshot guardado sin repetir el scan. Un snapshot parcial permite paginar las filas reunidas y señala `scanComplete=false`/`snapshotOnly=true`; no reanuda archivos no escaneados. El cursor dura como la referencia detail (10 min o sesión) y conserva el conjunto de resultados capturado. Patrones de código: query=fetch($URL) con ast=javascript (o lenguaje compatible); reconoce estructuras multilinea y devuelve hash/contexto sin índice. No combinar ast con regex/caseSensitive. Texto usa el motor de ripgrep: literal por defecto; si no hay coincidencias y la consulta tiene varias palabras sin puntuación, recupera términos y ordena con BM25 automáticamente (retrieval.mode=terms). Opciones explícitas regex/caseSensitive desactivan esa recuperación. regex=true habilita regex Rust por línea, caseSensitive=false ignora mayúsculas. onlyMatching=true extrae solo el fragmento coincidente (o el grupo de captura 1 cuando regex=true tiene paréntesis de captura) y devuelve una lista deduplicada en data.extracted. En kind=text, contextLines=0..20 devuelve matches[].hash (SHA-256 del archivo) y context:{start,end,text}: rango inclusivo base 1 y fuente exacta con saltos originales. Para harness_replace usa path=matches[].file, oldText=matches[].context.text y hash=matches[].hash, sin releer si el contexto es suficiente y no está recortado; el hash detecta cambios concurrentes."
    )]
    async fn harness_search(
        &self,
        Parameters(args): Parameters<SearchArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Search(args), context).await
    }

    #[tool(
        description = "Gestiona .harnessignore: action=list|add|remove|init; scope, patterns, reason."
    )]
    async fn harness_ignore(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::IgnoreArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Ignore(args), context).await
    }

    #[tool(description = "Inspecciona o lee un libro XLSX por rango o página.")]
    async fn harness_xlsx(
        &self,
        Parameters(args): Parameters<harness_domain::xlsx::XlsxArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(args.into_request(), context).await
    }

    #[tool(description = "Inspecciona o previsualiza SVG estático (≤256 KiB).")]
    async fn harness_svg(
        &self,
        Parameters(args): Parameters<harness_domain::svg::SvgArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Svg(args), context).await
    }

    #[tool(description = "Proyecta JSON con JMESPath (path|data|resultRef).")]
    async fn harness_data_query(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::DataQueryArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::DataQuery(args), context).await
    }
}

#[tool_handler(router = self.profile_router())]
impl<W, S> ServerHandler for McpServer<W, S>
where
    W: Workspace + 'static,
    S: SessionRepository + 'static,
{
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CallToolResponse, rmcp::ErrorData> {
        let tool = request.name.to_string();
        let call = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        self.profile_router()
            .call(call)
            .await
            .map(|response| presentation::normalize_text_only_error(response, &tool))
    }

    #[allow(deprecated)]
    async fn set_level(
        &self,
        request: rmcp::model::SetLevelRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<(), rmcp::ErrorData> {
        self.task_notifications.store(
            matches!(
                request.level,
                rmcp::model::LoggingLevel::Debug | rmcp::model::LoggingLevel::Info
            ),
            Ordering::Relaxed,
        );
        Ok(())
    }

    #[allow(deprecated)]
    async fn on_initialized(&self, context: rmcp::service::NotificationContext<RoleServer>) {
        let Some(tasks) = &self.tasks else {
            return;
        };
        if self.task_forwarding.swap(true, Ordering::AcqRel) {
            return;
        }
        let mut events = tasks.subscribe();
        let enabled = self.task_notifications.clone();
        tokio::spawn(async move {
            loop {
                let Some(event) = jobs::event_value(events.recv().await) else {
                    break;
                };
                if enabled.load(Ordering::Relaxed)
                    && context
                        .peer
                        .notify_logging_message(
                            rmcp::model::LoggingMessageNotificationParam::new(
                                rmcp::model::LoggingLevel::Info,
                                event,
                            )
                            .with_logger("harness.tasks"),
                        )
                        .await
                        .is_err()
                {
                    break;
                }
            }
        });
    }

    #[allow(deprecated)]
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_logging().build())
            .with_instructions(format!("Rust harness; fixed tool profile: {}. Start with harness_bootstrap, confirm data.root and reuse data.session. Bootstrap includes valid parameter examples; use structuredContent as the canonical response envelope and preserve diagnostics/detail. For calls, dependencies, and impact, use harness_explore first when a compatible index exists, even if global freshness is not certified or SYNC_WATCH_BUSY is reported. Inspect fileFreshness, coverage, truncation and absenceProven; verify uncertain or missing relations against source. SYNC_WATCH_BUSY only means this instance could not acquire the watcher lock; the owner and its health are unknown. Do not rebuild solely to query. Only advertised tools/operations are allowed; a profile is not permission to edit, run code or reindex without user authorization.", self.application.tool_profile().name()))
    }
}

#[cfg(test)]
#[path = "presentation_tests.rs"]
mod presentation_tests;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

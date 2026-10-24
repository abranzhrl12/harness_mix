//! MCP routing; native composition and execution lifecycle live in dedicated modules.
use harness_application::{
    Application,
    ports::{JobService, SessionRepository, Workspace},
};
mod execution;
mod input;
mod jobs;
mod presentation;
mod profile;
mod runtime;
use execution::execute;
use harness_domain::contracts::{
    BootstrapArgs, BrowserArgs, DetailArgs, ReadArgs, Request, SearchArgs, VerifyArgs,
};
use presentation::response;
#[cfg(test)]
use presentation::{response_with_compact, response_with_presentation};
use rmcp::{
    RoleServer, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ServerCapabilities, ServerConfig},
    service::RequestContext,
    tool, tool_handler, tool_router,
};
pub use runtime::{serve, serve_with_logging, serve_with_workspace_config};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

struct McpHarness<W, S> {
    application: Application<W, S>,
    tasks: Option<Arc<dyn JobService>>,
    task_notifications: Arc<AtomicBool>,
    task_forwarding: Arc<AtomicBool>,
}

impl<W, S> Clone for McpHarness<W, S> {
    fn clone(&self) -> Self {
        Self {
            application: self.application.clone(),
            tasks: self.tasks.clone(),
            task_notifications: self.task_notifications.clone(),
            task_forwarding: self.task_forwarding.clone(),
        }
    }
}

impl<W: Workspace + 'static, S: SessionRepository + 'static> McpHarness<W, S> {
    fn profile_router(&self) -> rmcp::handler::server::router::tool::ToolRouter<Self> {
        profile::router(Self::tool_router(), self.application.tool_profile())
    }
    async fn call(&self, request: Request, context: RequestContext<RoleServer>) -> CallToolResult {
        execute(&self.application, request, context.ct.cancelled()).await
    }
}

#[tool_router]
impl<W, S> McpHarness<W, S>
where
    W: Workspace + 'static,
    S: SessionRepository + 'static,
{
    #[tool(description = "Verificación en segundo plano: action=start|status|observe|cancel.")]
    async fn harness_jobs(
        &self,
        Parameters(args): Parameters<harness_domain::tasks::TaskArgs>,
    ) -> CallToolResult {
        jobs::handle(self.tasks.as_deref(), args).await
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
        description = "Conecta o inspecciona un navegador Chromium: action=connect|targets|attach|inspect|errors|screenshot|disconnect. Devuelve errores de consola/red, DOM y medidas del elemento seleccionado."
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
                return response(envelope);
            }
        }
        result
    }
    #[tool(description = "Git de solo lectura: action=status|diff.")]
    async fn harness_git(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::GitArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(args.into_request(), context).await
    }
    #[tool(description = "Relaciones de un símbolo: view=impact|summary|full.")]
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
        description = "Símbolos locales: outline admite filterKind=method y view=compact; read admite view=signature|full. HTML usa el parser Rust tl: outline devuelve elementos/atributos/padres y read acepta symbol=html:#guardar, html:button.action o html:[data-testid=guardar]. Selectores CSS embebidos conservan su nombre sin html:. full devuelve texto original y hash para editar; no valida Angular."
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
        description = "Compila o prueba con herramientas locales, sin shell. language=angular invoca @angular/compiler-cli local sin emitir archivos; project/configuration seleccionan angular.json."
    )]
    async fn harness_verify(
        &self,
        Parameters(args): Parameters<VerifyArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Verify(args), context).await
    }

    #[tool(
        description = "SQL de solo lectura: action=map|query|status|search|describe. map=esquema completo con PK, FK y parámetros en una llamada."
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
        description = "Lee hasta 420 líneas UTF-8 con hash y contexto. compact=true omite metadatos vacíos del sobre; lineNumbers=true añade data.numberedText de visualización sin alterar data.text."
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
        description = "Busca text/files/file_size/syntax con scope y globs. En kind=text, contextLines=0..20 devuelve matches[].hash (SHA-256 del archivo) y context:{start,end,text}: rango inclusivo base 1 y fuente exacta con saltos originales. Para harness_replace usa path=matches[].file, oldText=matches[].context.text y hash=matches[].hash, sin releer si el contexto es suficiente y no está recortado; el hash detecta cambios concurrentes."
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
impl<W, S> ServerHandler for McpHarness<W, S>
where
    W: Workspace + 'static,
    S: SessionRepository + 'static,
{
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
            .with_instructions(format!("Rust harness; fixed tool profile: {}. Start with harness_bootstrap, reuse data.session, then harness_explore for relationships. Only advertised tools/operations are allowed; a profile is not permission to edit, run code or reindex without user authorization.", self.application.tool_profile().name()))
    }
}

#[cfg(test)]
mod presentation_tests;
#[cfg(test)]
mod tests;

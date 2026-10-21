//! MCP routing; native composition and execution lifecycle live in dedicated modules.
use crate::adapters::NativeApplication;
mod execution;
mod input;
mod presentation;
mod profile;
mod runtime;
use execution::execute;
use harness_domain::contracts::{
    BootstrapArgs, DetailArgs, ReadArgs, Request, SearchArgs, VerifyArgs,
};
use presentation::response;
#[cfg(test)]
use presentation::response_with_presentation;
use rmcp::{
    RoleServer, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ServerCapabilities, ServerConfig},
    service::RequestContext,
    tool, tool_handler, tool_router,
};
pub use runtime::{serve, serve_with_logging};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone)]
struct McpHarness {
    application: NativeApplication,
    tasks: Option<crate::adapters::tasks::TaskManager>,
    task_notifications: Arc<AtomicBool>,
    task_forwarding: Arc<AtomicBool>,
}

impl McpHarness {
    fn profile_router(&self) -> rmcp::handler::server::router::tool::ToolRouter<Self> {
        profile::router(Self::tool_router(), self.application.tool_profile())
    }
    async fn call(&self, request: Request, context: RequestContext<RoleServer>) -> CallToolResult {
        execute(&self.application, request, context.ct.cancelled()).await
    }
}

#[tool_router]
impl McpHarness {
    #[tool(description = "Verificación en segundo plano: action=start|status|observe|cancel.")]
    async fn harness_jobs(
        &self,
        Parameters(args): Parameters<harness_domain::tasks::TaskArgs>,
    ) -> CallToolResult {
        let arguments = serde_json::to_value(&args).expect("task arguments");
        let result = match &self.tasks {
            Some(tasks) => tasks.handle(args).await,
            None => Err((
                "TOOL_PROFILE",
                "Background jobs are unavailable in this tool profile",
            )),
        };
        response(harness_domain::results::Results::default().envelope(
            "harness_jobs",
            &arguments,
            result,
        ))
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
    #[tool(description = "Ejecuta 1–4 lecturas independientes en paralelo.")]
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
        description = "Símbolos locales y selectores CSS (CSS/HTML/Razor): action=outline|read, view=signature|full."
    )]
    async fn harness_symbols(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::SymbolsArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(args.into_request(), context).await
    }
    #[tool(
        description = "Reemplaza oldText único o por line; hash opcional ligado a lectura interna; preview=true no publica. Acepta snake_case."
    )]
    async fn harness_replace(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::ReplaceArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Replace(args), context).await
    }
    #[tool(description = "Crea un archivo UTF-8; expectedHash reemplaza si coincide.")]
    async fn harness_write(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::WriteArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Write(args), context).await
    }
    #[tool(description = "Mueve o renombra un archivo dentro del workspace.")]
    async fn harness_move(
        &self,
        Parameters(args): Parameters<harness_domain::contracts::MoveArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Move(args), context).await
    }
    #[tool(description = "Compila o prueba con herramientas locales, sin shell.")]
    async fn harness_verify(
        &self,
        Parameters(args): Parameters<VerifyArgs>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        self.call(Request::Verify(args), context).await
    }

    #[tool(description = "SQL de solo lectura: action=query|status|search|describe.")]
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

    #[tool(description = "Lee hasta 400 líneas UTF-8 con hash y contexto.")]
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
        description = "Busca text/files/file_size/syntax con scope y globs; contextLines=0..20 añade fuente exacta y hash en kind=text."
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
impl ServerHandler for McpHarness {
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
                let event = match events.recv().await {
                    Ok(event) => event,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        serde_json::json!({"kind":"events_lagged","skipped":skipped,"recovery":"harness_jobs action=events"})
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
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

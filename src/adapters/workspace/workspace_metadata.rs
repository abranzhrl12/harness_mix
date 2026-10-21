use crate::adapters::{NativeWorkspace, capabilities, config, external_scopes};
use harness_application::ports::workspace::WorkspaceMetadata;
use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;
use serde_json::json;

impl WorkspaceMetadata for NativeWorkspace {
    fn external_scope(
        &self,
        args: &harness_domain::contracts::ExternalScopeArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        use harness_domain::contracts::ExternalScopeAction;
        context.check()?;
        match args.action {
            ExternalScopeAction::Grant => {
                if args.user_authorized != Some(true) {
                    return Err((
                        "AUTHORIZATION",
                        "Grant requires explicit user consent for this exact path and task",
                    ));
                }
                let approved = external_scopes::approve(
                    args.root
                        .as_deref()
                        .ok_or(("ARGUMENT", "grant requires root"))?,
                    context,
                )?;
                self.external_scopes
                    .lock()
                    .map_err(|_| ("INTERNAL", "External registry unavailable"))?
                    .grant(&args.session, approved, context)
            }
            ExternalScopeAction::List => Ok(self
                .external_scopes
                .lock()
                .map_err(|_| ("INTERNAL", "External registry unavailable"))?
                .list(&args.session)),
            ExternalScopeAction::Revoke => self
                .external_scopes
                .lock()
                .map_err(|_| ("INTERNAL", "External registry unavailable"))?
                .revoke(
                    &args.session,
                    args.scope_id
                        .as_deref()
                        .ok_or(("ARGUMENT", "revoke requires scopeId"))?,
                ),
        }
    }
    fn config(
        &self,
        args: &harness_domain::contracts::ConfigArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        if args.external_scope.is_some() {
            return self.read_project(
                args.external_scope.as_deref(),
                args.session.as_deref(),
                context,
                |project, _| config::section(project, args, context),
            );
        }
        config::section(&self.project, args, context)
    }
    fn info(&self) -> Value {
        let root = self.project.root.to_string_lossy();
        let root = root.strip_prefix(r"\\?\").unwrap_or(&root);
        let manifests = [
            ("rust", "Cargo.toml"),
            ("go", "go.mod"),
            ("javascript", "package.json"),
            ("python", "pyproject.toml"),
            ("java", "pom.xml"),
            ("dotnet", "global.json"),
        ];
        let mut languages: Vec<&str> = manifests
            .iter()
            .filter(|(_, file)| self.project.root.join(file).is_file())
            .map(|(language, _)| *language)
            .collect();
        if !languages.contains(&"python") && self.project.root.join("requirements.txt").is_file() {
            languages.push("python");
        }
        if !languages.contains(&"dotnet")
            && std::fs::read_dir(&self.project.root)
                .ok()
                .is_some_and(|items| {
                    items.filter_map(Result::ok).any(|entry| {
                        entry.path().extension().is_some_and(|ext| {
                            ["csproj", "sln", "slnx"]
                                .iter()
                                .any(|s| ext.eq_ignore_ascii_case(s))
                        })
                    })
                })
        {
            languages.push("dotnet");
        }
        json!({"root":root,"languages":languages,"implementationVersion":env!("CARGO_PKG_VERSION"),"manifestScope":"raíz del proyecto","indexed":self.project.root.join(".codegraph").is_dir()})
    }
    fn rules(&self, context: &OperationContext) -> Result<String, Failure> {
        let relative = ".agents/rules/00-mandatory-tools.md";
        if !self.project.root.join(relative).exists() {
            return Ok(String::new());
        }
        String::from_utf8(self.project.bytes_with_context(relative, context)?)
            .map_err(|_| ("ENCODING", "reglas no UTF-8"))
    }
    fn capabilities(&self) -> Value {
        capabilities::capabilities()
    }
}

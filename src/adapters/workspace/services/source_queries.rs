use super::SourceQueryService;
use crate::adapters::{explore, project::numbered_text, search, symbols};
use harness_application::ports::workspace::SourceQueries;
use harness_domain::{
    Failure,
    contracts::{ReadArgs, SearchArgs},
    execution::OperationContext,
};
use serde_json::{Value, json};

#[cfg(test)]
#[path = "source_queries_tests.rs"]
mod tests;

impl SourceQueries for SourceQueryService {
    fn lsp(&self, args: &harness_domain::lsp::LspArgs, context: &OperationContext) -> Result<Value, Failure> {
        context.check()?;
        harness_lsp::language(&args.path)?;
        let bytes = self.project.symbol_bytes_with_context(&args.path, context)?;
        if bytes.len() > 2 * 1024 * 1024 { return Err(("SIZE", "LSP document exceeds 2 MiB")); }
        let text = String::from_utf8(bytes).map_err(|_| ("ENCODING", "LSP requires UTF-8 source"))?;
        let client = self.lsp.get_or_init(|| {
            let commands = match std::env::var("HARNESS_LSP_SERVERS") {
                Ok(value) => serde_json::from_str::<harness_lsp::Commands>(&value)
                    .map_err(|_| ("LSP_CONFIG", "HARNESS_LSP_SERVERS must be a JSON object mapping languages to command/args"))?,
                Err(std::env::VarError::NotPresent) => harness_lsp::default_commands(),
                Err(_) => return Err(("LSP_CONFIG", "HARNESS_LSP_SERVERS must be valid Unicode")),
            };
            if commands.len() > 4 || commands.iter().any(|(language, command)| {
                !matches!(language.as_str(), "rust" | "go" | "typescript" | "javascript")
                    || command.command.is_empty() || command.command.len() > 4096 || command.command.contains(['\r', '\n', '\0'])
                    || command.args.len() > 32 || command.args.iter().any(|arg| arg.len() > 4096 || arg.contains('\0'))
            }) { return Err(("LSP_CONFIG", "Invalid operator LSP command configuration")); }
            Ok(harness_lsp::Client::new(&self.project.root, commands))
        }).as_ref().map_err(|failure| *failure)?;
        client.query(args, text, context)
    }
    fn explore(
        &self,
        args: &harness_domain::contracts::ExploreArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        explore::explore(&self.project, args, context)
    }
    fn outline(
        &self,
        args: &harness_domain::contracts::OutlineArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        super::read_scoped_project(
            &self.project,
            &self.external_scopes,
            args.external_scope.as_deref(),
            args.session.as_deref(),
            context,
            |project, _| symbols::outline(project, args, context),
        )
    }
    fn read_symbol(
        &self,
        args: &harness_domain::contracts::ReadSymbolArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        super::read_scoped_project(
            &self.project,
            &self.external_scopes,
            args.external_scope.as_deref(),
            args.session.as_deref(),
            context,
            |project, _| symbols::read_symbol(project, args, context),
        )
    }
    fn read(&self, args: &ReadArgs, context: &OperationContext) -> Result<Value, Failure> {
        super::read_scoped_project(
            &self.project,
            &self.external_scopes,
            args.external_scope.as_deref(),
            args.session.as_deref(),
            context,
            |project, _| {
                let mut value =
                    project.read_with_context(&args.path, args.start, args.end, context)?;
                if args.line_numbers {
                    let start = value["start"].as_u64().unwrap_or(args.start as u64) as usize;
                    let end = value["end"].as_u64().unwrap_or(start as u64) as usize;
                    if let Some(text) = value["text"].as_str().map(str::to_owned) {
                        value["numberedText"] = json!(numbered_text(&text, start, end));
                    }
                }
                Ok(value)
            },
        )
    }
    fn resume_search(&self, args: &SearchArgs, checkpoint: &Value, context: &OperationContext) -> Result<Value, Failure> {
        super::read_scoped_project(
            &self.project, &self.external_scopes, args.external_scope.as_deref(),
            args.session.as_deref(), context,
            |project, default_scope| {
                if args.scope.is_none() && default_scope.is_some() {
                    let mut value = serde_json::to_value(args).map_err(|_| ("INTERNAL", "Cannot encode search arguments"))?;
                    value["scope"] = json!(default_scope);
                    let scoped = serde_json::from_value(value).map_err(|_| ("INTERNAL", "Cannot scope search arguments"))?;
                    search::search_with_resume(project, &scoped, context, Some(checkpoint))
                } else {
                    search::search_with_resume(project, args, context, Some(checkpoint))
                }
            },
        )
    }
    fn search(&self, args: &SearchArgs, context: &OperationContext) -> Result<Value, Failure> {
        super::read_scoped_project(
            &self.project,
            &self.external_scopes,
            args.external_scope.as_deref(),
            args.session.as_deref(),
            context,
            |project, default_scope| {
                if args.scope.is_none() && default_scope.is_some() {
                    let mut value = serde_json::to_value(args)
                        .map_err(|_| ("INTERNAL", "Cannot encode search arguments"))?;
                    value["scope"] = json!(default_scope);
                    let scoped = serde_json::from_value(value)
                        .map_err(|_| ("INTERNAL", "Cannot scope search arguments"))?;
                    search::search_with_context(project, &scoped, context)
                } else {
                    search::search_with_context(project, args, context)
                }
            },
        )
    }
}

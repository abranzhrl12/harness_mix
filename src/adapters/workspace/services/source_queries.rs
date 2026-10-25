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

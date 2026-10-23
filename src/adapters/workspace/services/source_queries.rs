use super::SourceQueryService;
use crate::adapters::{explore, project::numbered_text, search, symbols};
use harness_application::ports::workspace::SourceQueries;
use harness_domain::{
    Failure,
    contracts::{ReadArgs, SearchArgs},
    execution::OperationContext,
};
use serde_json::{Value, json};

impl SourceQueries for SourceQueryService {
    fn explore(
        &self,
        args: &harness_domain::contracts::ExploreArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        if args.sync_index == Some(true) {
            let _guard = self
                .edit_lock
                .lock()
                .map_err(|_| ("INTERNAL", "Workspace mutation lock unavailable"))?;
            context.check()?;
            explore::synchronized(&self.project, args, context)
        } else {
            explore::explore(&self.project, args, context)
        }
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
                let mut value = project.read_with_context(&args.path, args.start, args.end, context)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{Project, external_scopes::Registry, project::hash};
    use std::{fs, sync::{Arc, Mutex}};

    #[test]
    fn read_line_numbers_are_display_only_and_respect_the_selected_eof_line() {
        let dir = tempfile::tempdir().unwrap();
        let source = b"first\r\nsecond\r\n";
        fs::write(dir.path().join("example.txt"), source).unwrap();
        let service = SourceQueryService::new(
            Project::new(dir.path()).unwrap(),
            Arc::new(Mutex::new(Registry::default())),
            Arc::new(Mutex::new(())),
        );
        let context = OperationContext::default();
        let read_args = |line_numbers| ReadArgs {
            external_scope: None,
            path: "example.txt".into(),
            start: 2,
            end: 3,
            compact: false,
            line_numbers,
            session: None,
        };

        let numbered = service.read(&read_args(true), &context).unwrap();
        assert_eq!(numbered["text"], "second\r\n");
        assert_eq!(numbered["numberedText"], "2 | second\r\n3 | ");
        assert_eq!(numbered["hash"], hash(source));

        let plain = service.read(&read_args(false), &context).unwrap();
        assert_eq!(plain["text"], numbered["text"]);
        assert_eq!(plain["hash"], numbered["hash"]);
        assert!(plain.get("numberedText").is_none());
    }
}

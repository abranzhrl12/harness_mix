use crate::adapters::NativeWorkspace;
use harness_application::ports::workspace::SourceQueries;
use harness_domain::{
    Failure,
    contracts::{ReadArgs, SearchArgs},
    execution::OperationContext,
};
use serde_json::Value;

impl SourceQueries for NativeWorkspace {
    fn lsp(&self, args: &harness_domain::lsp::LspArgs, context: &OperationContext) -> Result<Value, Failure> {
        self.source_queries.lsp(args, context)
    }
    fn explore(
        &self,
        args: &harness_domain::contracts::ExploreArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        if args.sync_index == Some(true) {
            self.index_maintenance.explore_synchronized(args, context)
        } else {
            self.source_queries.explore(args, context)
        }
    }
    fn outline(
        &self,
        args: &harness_domain::contracts::OutlineArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        self.source_queries.outline(args, context)
    }
    fn read_symbol(
        &self,
        args: &harness_domain::contracts::ReadSymbolArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        self.source_queries.read_symbol(args, context)
    }
    fn read(&self, args: &ReadArgs, context: &OperationContext) -> Result<Value, Failure> {
        self.source_queries.read(args, context)
    }
    fn search(&self, args: &SearchArgs, context: &OperationContext) -> Result<Value, Failure> {
        self.source_queries.search(args, context)
    }
    fn resume_search(&self, args: &SearchArgs, checkpoint: &Value, context: &OperationContext) -> Result<Value, Failure> {
        self.source_queries.resume_search(args, checkpoint, context)
    }
}

use crate::adapters::NativeWorkspace;
use harness_application::ports::workspace::SourceEditing;
use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

impl SourceEditing for NativeWorkspace {
    fn prepare_proposal(
        &self,
        args: &harness_domain::contracts::ProposalArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        self.source_editing.prepare_proposal(args, context)
    }
    fn delete_file(
        &self,
        args: &harness_domain::contracts::DeleteArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        self.source_editing.delete_file(args, context)
    }
    fn vue_extract_preview(
        &self,
        args: &harness_domain::contracts::VueExtractPreviewArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        self.source_editing.vue_extract_preview(args, context)
    }
    fn patch_preview(
        &self,
        args: &harness_domain::contracts::ReplaceArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        self.source_editing.patch_preview(args, context)
    }
    fn replace(
        &self,
        args: &harness_domain::contracts::ReplaceArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        self.source_editing.replace(args, context)
    }
    fn write(
        &self,
        args: &harness_domain::contracts::WriteArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        self.source_editing.write(args, context)
    }
    fn move_file(
        &self,
        args: &harness_domain::contracts::MoveArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        self.source_editing.move_file(args, context)
    }
    fn ignore(
        &self,
        args: &harness_domain::contracts::IgnoreArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        self.source_editing.ignore(args, context)
    }
}

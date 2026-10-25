use crate::adapters::NativeWorkspace;
use harness_application::ports::workspace::{IgnoreManagement, SourceEditing, SourcePreviews};
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
        SourceEditing::vue_extract_preview(&self.source_editing, args, context)
    }
    fn patch_preview(
        &self,
        args: &harness_domain::contracts::ReplaceArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        SourceEditing::patch_preview(&self.source_editing, args, context)
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
        SourceEditing::ignore(&self.source_editing, args, context)
    }
}

impl SourcePreviews for NativeWorkspace {
    fn vue_extract_preview(
        &self,
        args: &harness_domain::contracts::VueExtractPreviewArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        SourcePreviews::vue_extract_preview(&self.source_editing, args, context)
    }
    fn patch_preview(
        &self,
        args: &harness_domain::contracts::ReplaceArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        SourcePreviews::patch_preview(&self.source_editing, args, context)
    }
}

impl IgnoreManagement for NativeWorkspace {
    fn ignore(
        &self,
        args: &harness_domain::contracts::IgnoreArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        IgnoreManagement::ignore(&self.source_editing, args, context)
    }
}

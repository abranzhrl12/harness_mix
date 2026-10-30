use super::SourceEditingService;
use crate::adapters::{edit, ignore, move_file, vue_extract};
use harness_application::ports::workspace::{IgnoreManagement, SourceEditing, SourcePreviews};
use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

impl SourceEditing for SourceEditingService {
    fn prepare_proposal(
        &self,
        args: &harness_domain::contracts::ProposalArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        let _lock = self
            .edit_lock
            .try_lock()
            .map_err(|_| ("EDIT_BUSY", "Another file operation is active"))?;
        crate::adapters::proposals::prepare(&self.project, args, context)
    }
    fn delete_file(
        &self,
        args: &harness_domain::contracts::DeleteArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        let _lock = self
            .edit_lock
            .try_lock()
            .map_err(|_| ("EDIT_BUSY", "Another file operation is active"))?;
        crate::adapters::delete_file::delete_file(&self.project, args, context)
    }
    fn vue_extract_preview(
        &self,
        args: &harness_domain::contracts::VueExtractPreviewArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        vue_extract::extract_preview(&self.project, args, context)
    }
    fn patch_preview(
        &self,
        args: &harness_domain::contracts::ReplaceArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        edit::preview(&self.project, args, context)
    }
    fn replace(
        &self,
        args: &harness_domain::contracts::ReplaceArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        let _lock = self.edit_lock.try_lock().map_err(|_| {
            (
                "EDIT_BUSY",
                "Another edit is active; read again before retrying",
            )
        })?;
        edit::replace(&self.project, args, context)
    }
    fn write(
        &self,
        args: &harness_domain::contracts::WriteArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        let _lock = self
            .edit_lock
            .try_lock()
            .map_err(|_| ("EDIT_BUSY", "Another edit is active"))?;
        edit::write(&self.project, args, context)
    }
    fn write_word(
        &self,
        args: &harness_domain::contracts::WordArgs,
        bytes: &[u8],
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        let _lock = self
            .edit_lock
            .try_lock()
            .map_err(|_| ("EDIT_BUSY", "Another file operation is active"))?;
        crate::adapters::create_file::write_word(&self.project, args, bytes, context)
    }
    fn move_file(
        &self,
        args: &harness_domain::contracts::MoveArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        let _lock = self.edit_lock.try_lock().map_err(|_| {
            (
                "EDIT_BUSY",
                "Another edit or move is active; retry after it completes",
            )
        })?;
        move_file::move_file(&self.project, args, context)
    }
    fn ignore(
        &self,
        args: &harness_domain::contracts::IgnoreArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        ignore::ignore(&self.project, args, context)
    }
}

impl SourcePreviews for SourceEditingService {
    fn vue_extract_preview(
        &self,
        args: &harness_domain::contracts::VueExtractPreviewArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        vue_extract::extract_preview(&self.project, args, context)
    }
    fn patch_preview(
        &self,
        args: &harness_domain::contracts::ReplaceArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        edit::preview(&self.project, args, context)
    }
}

impl IgnoreManagement for SourceEditingService {
    fn ignore(
        &self,
        args: &harness_domain::contracts::IgnoreArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        ignore::ignore(&self.project, args, context)
    }
}

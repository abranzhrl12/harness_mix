use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

/// Source editing and previews; implementations retain publication and path safety.
pub trait SourceEditing: Send + Sync {
    fn prepare_proposal(
        &self,
        _args: &harness_domain::contracts::ProposalArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Proposal adapter unavailable"))
    }
    fn delete_file(
        &self,
        _args: &harness_domain::contracts::DeleteArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Delete adapter unavailable"))
    }
    fn vue_extract_preview(
        &self,
        _args: &harness_domain::contracts::VueExtractPreviewArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Vue extraction preview adapter unavailable"))
    }
    fn patch_preview(
        &self,
        _args: &harness_domain::contracts::ReplaceArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Patch preview adapter unavailable"))
    }
    fn replace(
        &self,
        _args: &harness_domain::contracts::ReplaceArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Editing adapter unavailable"))
    }
    fn move_file(
        &self,
        _args: &harness_domain::contracts::MoveArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Move adapter unavailable"))
    }
    fn write(
        &self,
        _args: &harness_domain::contracts::WriteArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "File creation adapter unavailable"))
    }
    fn ignore(
        &self,
        _args: &harness_domain::contracts::IgnoreArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Ignore policy adapter unavailable"))
    }
}

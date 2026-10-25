use harness_domain::{
    Failure,
    contracts::{
        DeleteArgs, IgnoreArgs, MoveArgs, ProposalArgs, ReplaceArgs, VueExtractPreviewArgs,
        WriteArgs,
    },
    execution::OperationContext,
};
use serde_json::Value;

use crate::ports::workspace::SourceEditing;

pub fn delete<S: SourceEditing + ?Sized>(
    service: &S,
    args: &DeleteArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    service.delete_file(args, context)
}

pub fn replace<S: SourceEditing + ?Sized>(
    service: &S,
    args: &ReplaceArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    service.replace(args, context)
}

pub fn write<S: SourceEditing + ?Sized>(
    service: &S,
    args: &WriteArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    service.write(args, context)
}

pub fn move_file<S: SourceEditing + ?Sized>(
    service: &S,
    args: &MoveArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    service.move_file(args, context)
}

pub fn ignore<S: SourceEditing + ?Sized>(
    service: &S,
    args: &IgnoreArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    service.ignore(args, context)
}

pub fn patch_preview<S: SourceEditing + ?Sized>(
    service: &S,
    args: &ReplaceArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    service.patch_preview(args, context)
}

pub fn prepare_proposal<S: SourceEditing + ?Sized>(
    service: &S,
    args: &ProposalArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    service.prepare_proposal(args, context)
}

pub fn vue_extract_preview<S: SourceEditing + ?Sized>(
    service: &S,
    args: &VueExtractPreviewArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    service.vue_extract_preview(args, context)
}

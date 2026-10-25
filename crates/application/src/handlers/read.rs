use harness_domain::{
    Failure,
    contracts::{ExploreArgs, OutlineArgs, ReadArgs, ReadSymbolArgs},
    execution::OperationContext,
};
use serde_json::Value;

use crate::ports::workspace::SourceQueries;

/// Read a source range using the read-only query capability.
pub fn read<S: SourceQueries + ?Sized>(
    service: &S,
    args: &ReadArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    service.read(args, context)
}

/// Read a source outline using the read-only query capability.
pub fn outline<S: SourceQueries + ?Sized>(
    service: &S,
    args: &OutlineArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    service.outline(args, context)
}

/// Read a symbol implementation using the read-only query capability.
pub fn read_symbol<S: SourceQueries + ?Sized>(
    service: &S,
    args: &ReadSymbolArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    service.read_symbol(args, context)
}

/// Explore relationships and apply the existing impact response projection.
pub fn explore<S: SourceQueries + ?Sized>(
    service: &S,
    args: &ExploreArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    let impact = args.view == Some(harness_domain::contracts::ExploreView::Impact);
    service.explore(args, context).map(|result| {
        if impact {
            harness_domain::assistance::impact(args, result)
        } else {
            result
        }
    })
}

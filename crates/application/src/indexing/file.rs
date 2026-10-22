//! Explicit file indexing lifecycle, including stages before transaction creation.
use crate::ports::indexing::FileIndexPort;
use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

pub fn sync_file<P: FileIndexPort>(
    port: &mut P,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    port.validate_request(context)?;
    context.check()?;
    port.load_scope(context)?;
    context.check()?;
    port.read_source(context)?;
    context.check()?;
    let tx = port.begin(context)?;
    super::transaction::execute(tx, context)
}

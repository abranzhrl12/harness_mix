//! Atomic file lifecycle. Never test cancellation after a confirmed commit.
use crate::ports::indexing::FileTransactionPort;
use harness_domain::{Failure, execution::OperationContext};

pub fn execute<T: FileTransactionPort>(
    mut tx: T,
    context: &OperationContext,
) -> Result<T::Output, Failure> {
    let prepared = (|| {
        context.check()?;
        tx.validate(context)?;
        context.check()?;
        let publication = tx.publish(context)?;
        context.check()?;
        tx.recheck(&publication, context)?;
        context.check()?;
        tx.integrity(&publication, context)?;
        context.check()?;
        Ok(publication)
    })();
    match prepared {
        Err(failure) => {
            tx.rollback()?;
            Err(context.check().err().unwrap_or(failure))
        }
        Ok(publication) => tx.commit(publication),
    }
}

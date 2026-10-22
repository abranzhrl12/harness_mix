//! Indexing contracts. Storage sessions and request models are supplied by adapters;
//! this module must not depend on SQLite, extraction engines or the filesystem.
use harness_domain::{Failure, execution::OperationContext};

/// Runs within the caller's atomic publication. Implementations must preserve
/// cross-file retries, propagate cancellation/failures and never commit the session.
/// A deferred implementation is not substitutable until its final retry pass exists.
pub trait ReferenceInvalidatorPort<Session, Scope> {
    type Outcome;

    fn invalidate(
        &self,
        session: &Session,
        context: &OperationContext,
        scope: Scope,
    ) -> Result<Self::Outcome, Failure>;
}

/// An already-open atomic publication. Concrete transaction, parser, snapshots and
/// resolver caches never cross into application. The owner commits only after the
/// use case AND source/integrity revalidation succeed; any error must roll back.
pub trait PublicationSession {
    type Extraction;
    type Captured;
    type Written;
    type Invalidation;
    type Resolution;
    type Output;

    fn stage(&mut self, stage: crate::indexing::PublicationStage);
    fn unchanged(&mut self, context: &OperationContext) -> Result<Option<Self::Output>, Failure>;
    fn extract(&mut self, context: &OperationContext) -> Result<Self::Extraction, Failure>;
    fn capture(
        &mut self,
        extraction: &Self::Extraction,
        context: &OperationContext,
    ) -> Result<Self::Captured, Failure>;
    fn remove_previous(
        &mut self,
        captured: &Self::Captured,
        context: &OperationContext,
    ) -> Result<(), Failure>;
    fn write(
        &mut self,
        extraction: &Self::Extraction,
        captured: &Self::Captured,
        context: &OperationContext,
    ) -> Result<Self::Written, Failure>;
    fn restore(
        &mut self,
        captured: &Self::Captured,
        written: &mut Self::Written,
        context: &OperationContext,
    ) -> Result<(), Failure>;
    fn invalidate(
        &mut self,
        captured: &Self::Captured,
        written: &mut Self::Written,
        context: &OperationContext,
    ) -> Result<Self::Invalidation, Failure>;
    fn resolve(
        &mut self,
        written: &Self::Written,
        invalidation: &Self::Invalidation,
        context: &OperationContext,
    ) -> Result<Self::Resolution, Failure>;
    fn recheck(
        &mut self,
        resolution: &Self::Resolution,
        context: &OperationContext,
    ) -> Result<(), Failure>;
    fn metadata(
        &mut self,
        captured: &Self::Captured,
        resolution: &Self::Resolution,
        context: &OperationContext,
    ) -> Result<(), Failure>;
    fn finish(
        &mut self,
        extraction: Self::Extraction,
        captured: Self::Captured,
        written: Self::Written,
        invalidation: Self::Invalidation,
        resolution: Self::Resolution,
    ) -> Result<Self::Output, Failure>;
}

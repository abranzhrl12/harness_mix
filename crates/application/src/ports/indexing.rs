//! Indexing contracts. Storage sessions and request models are supplied by adapters;
//! this module must not depend on SQLite, extraction engines or the filesystem.
use harness_domain::{Failure, execution::OperationContext};

/// Infrastructure capabilities used by the workspace use case. Preparation is
/// optional and positional: one slot per requested path, with no database writes.
/// apply owns a per-file atomic transaction; success must confirm its commit.
pub trait WorkspacePort {
    type Prepared;
    fn load_scope(&mut self, context: &OperationContext) -> Result<serde_json::Value, Failure>;
    fn plan(&mut self, context: &OperationContext) -> Result<serde_json::Value, Failure>;
    fn language(&self, path: &str) -> Option<&'static str>;
    fn workers(&self) -> usize;
    fn prepare(
        &mut self,
        paths: &[&String],
        context: &OperationContext,
    ) -> Vec<Option<Self::Prepared>>;
    fn apply(
        &mut self,
        path: &str,
        prepared: Option<Self::Prepared>,
        force: bool,
        context: &OperationContext,
    ) -> Result<serde_json::Value, Failure>;
    fn diagnostic(&mut self, code: &str, message: &str) -> Option<serde_json::Value>;
    /// False when the last apply error invalidated a workspace precondition,
    /// rather than one file. Systemic error codes still stop even if true.
    /// File errors must have a confirmed rollback or use SYNC_COMMIT_UNCERTAIN.
    fn file_failure_is_isolated(&self) -> bool {
        true
    }
    /// Mark a rolled-back forced rebuild as dirty so a later incremental plan retries it.
    /// Returns false when the path has no prior indexed row.
    fn mark_failed_rebuild(
        &mut self,
        _path: &str,
        _context: &OperationContext,
    ) -> Result<bool, Failure> {
        Ok(false)
    }
    fn recheck(&mut self, context: &OperationContext) -> Result<(), Failure>;
}

/// An opened file transaction. All error paths must release/roll back writes,
/// even if the operation context is cancelled. commit consumes ownership; its
/// failure must report uncertain durability rather than pretend no write occurred.
pub trait FileTransactionPort: Sized {
    type Publication;
    type Output;
    fn validate(&mut self, context: &OperationContext) -> Result<(), Failure>;
    fn publish(&mut self, context: &OperationContext) -> Result<Self::Publication, Failure>;
    fn recheck(
        &mut self,
        publication: &Self::Publication,
        context: &OperationContext,
    ) -> Result<(), Failure>;
    fn integrity(
        &mut self,
        publication: &Self::Publication,
        context: &OperationContext,
    ) -> Result<(), Failure>;
    fn rollback(self) -> Result<(), Failure>;
    fn commit(self, publication: Self::Publication) -> Result<Self::Output, Failure>;
}

/// Pre-transaction file stages. No transaction is opened if validation or source
/// capture fails. The returned transaction borrows its captured source/scope owner.
pub trait FileIndexPort {
    type Transaction<'a>: FileTransactionPort<Output = serde_json::Value>
    where
        Self: 'a;
    fn validate_request(&mut self, context: &OperationContext) -> Result<(), Failure>;
    fn load_scope(&mut self, context: &OperationContext) -> Result<(), Failure>;
    fn read_source(&mut self, context: &OperationContext) -> Result<(), Failure>;
    fn begin(&mut self, context: &OperationContext) -> Result<Self::Transaction<'_>, Failure>;
}

pub enum SourceMetadata {
    Missing,
    Regular(u64),
    Other,
}

pub struct DiscoveredEntry {
    pub path: Option<String>,
    pub is_file: bool,
    pub scan_error: bool,
}

/// A lazy discovery stream avoids buffering an unbounded tree. Implementations
/// enforce path security and retain OS-specific pins for the complete source read.
pub trait InventoryPort {
    fn scope(&self) -> serde_json::Value;
    fn has_includes(&self) -> bool;
    fn excluded(&self, path: &str) -> bool;
    fn included(&self, path: &str) -> bool;
    fn restricted_reason(&self, path: &str) -> Option<&'static str>;
    fn supported(&self, path: &str) -> bool;
    fn walk(&self, include_pass: bool) -> Box<dyn Iterator<Item = DiscoveredEntry>>;
    fn metadata(&self, path: &str) -> Result<SourceMetadata, (String, String)>;
    fn read(&self, path: &str, limit: u64, context: &OperationContext) -> Result<Vec<u8>, Failure>;
    fn recheck(&self, context: &OperationContext) -> Result<(), Failure>;
}

pub trait ChangedPathsPort {
    fn initialize(&mut self, context: &OperationContext) -> Result<(), Failure>;
    /// Worktree ignore semantics may require a conservative full inventory.
    fn needs_inventory(&self, paths: &std::collections::BTreeSet<String>) -> Result<bool, Failure>;
    fn sync_workspace(&mut self, context: &OperationContext) -> Result<serde_json::Value, Failure>;
    fn apply_changed(
        &mut self,
        path: &str,
        context: &OperationContext,
    ) -> Result<Option<serde_json::Value>, Failure>;
    /// Same isolation contract as WorkspacePort::file_failure_is_isolated.
    fn file_failure_is_isolated(&self) -> bool {
        true
    }
}

pub trait PlanningPort {
    fn indexed(
        &mut self,
        context: &OperationContext,
    ) -> Result<std::collections::BTreeMap<String, (String, bool)>, Failure>;
    fn inventory(
        &mut self,
        tracked: &[String],
        context: &OperationContext,
    ) -> Result<crate::indexing::inventory::Inventory, Failure>;
    fn restricted(&self, path: &str) -> bool;
    fn preview(
        &mut self,
        plan: &crate::indexing::planning::Plan,
        inventory: &crate::indexing::inventory::Inventory,
        context: &OperationContext,
    ) -> Result<serde_json::Value, Failure>;
}

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

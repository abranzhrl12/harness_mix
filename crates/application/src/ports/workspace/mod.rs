//! Capability-specific application ports. The aggregate is only a composition bound.
mod artifact_queries;
mod log_persistence;
mod session_persistence;
mod source_editing;
mod source_queries;
mod sql_access;
mod verification;
mod version_control;
mod workspace_diagnostics;
mod workspace_metadata;

pub use artifact_queries::ArtifactQueries;
pub use log_persistence::LogPersistence;
pub use session_persistence::SessionPersistence;
pub use source_editing::{IgnoreManagement, SourceEditing, SourcePreviews};
pub use source_queries::SourceQueries;
pub use sql_access::SqlAccess;
pub use verification::Verification;
pub use version_control::VersionControl;
pub use workspace_diagnostics::WorkspaceDiagnostics;
pub use workspace_metadata::WorkspaceMetadata;

/// Complete tool-host composition; individual use cases may depend on one capability.
pub trait Workspace:
    WorkspaceMetadata
    + super::browser::BrowserInspection
    + SourceQueries
    + SourceEditing
    + SourcePreviews
    + IgnoreManagement
    + WorkspaceDiagnostics
    + VersionControl
    + SqlAccess
    + ArtifactQueries
    + SessionPersistence
    + LogPersistence
    + Verification
{
}

impl<T: ?Sized> Workspace for T where
    T: WorkspaceMetadata
        + super::browser::BrowserInspection
        + SourceQueries
        + SourceEditing
        + SourcePreviews
        + IgnoreManagement
        + WorkspaceDiagnostics
        + VersionControl
        + SqlAccess
        + ArtifactQueries
        + SessionPersistence
        + LogPersistence
        + Verification
{
}

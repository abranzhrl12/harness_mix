//! Indexing use cases, independent of persistence and extraction.
pub mod changed_paths;
pub mod file;
pub mod inventory;
pub mod planning;
mod profile;
pub mod transaction;
pub mod workspace;
use crate::ports::indexing::PublicationSession;
use harness_domain::{Failure, execution::OperationContext};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationStage {
    CheckIndexedHash,
    ExtractSource,
    CaptureExistingGraph,
    DeletePreviousGraph,
    WriteNodesAndEdges,
    RestoreIncomingEdges,
    InvalidateReferences,
    ResolveReferences,
    RecheckFrameworkSources,
    WriteMetadata,
}

impl PublicationStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CheckIndexedHash => "check_indexed_hash",
            Self::ExtractSource => "extract_source",
            Self::CaptureExistingGraph => "capture_existing_graph",
            Self::DeletePreviousGraph => "delete_previous_graph",
            Self::WriteNodesAndEdges => "write_nodes_and_edges",
            Self::RestoreIncomingEdges => "restore_incoming_edges",
            Self::InvalidateReferences => "invalidate_references",
            Self::ResolveReferences => "resolve_references",
            Self::RecheckFrameworkSources => "recheck_framework_sources",
            Self::WriteMetadata => "write_metadata",
        }
    }
}

/// Coordinates each publication step without knowing storage or parser types.
/// Never commits: the transaction owner must still revalidate source snapshots.
pub fn publish<S: PublicationSession>(
    session: &mut S,
    context: &OperationContext,
) -> Result<S::Output, Failure> {
    fn stage<S: PublicationSession>(
        session: &mut S,
        context: &OperationContext,
        stage: PublicationStage,
    ) -> Result<(), Failure> {
        context.check()?;
        session.stage(stage);
        context.check()
    }
    stage(session, context, PublicationStage::CheckIndexedHash)?;
    if let Some(output) = session.unchanged(context)? {
        return Ok(output);
    }
    stage(session, context, PublicationStage::ExtractSource)?;
    let extraction = session.extract(context)?;
    stage(session, context, PublicationStage::CaptureExistingGraph)?;
    let captured = session.capture(&extraction, context)?;
    stage(session, context, PublicationStage::DeletePreviousGraph)?;
    session.remove_previous(&captured, context)?;
    stage(session, context, PublicationStage::WriteNodesAndEdges)?;
    let mut written = session.write(&extraction, &captured, context)?;
    stage(session, context, PublicationStage::RestoreIncomingEdges)?;
    session.restore(&captured, &mut written, context)?;
    stage(session, context, PublicationStage::InvalidateReferences)?;
    let invalidation = session.invalidate(&captured, &mut written, context)?;
    stage(session, context, PublicationStage::ResolveReferences)?;
    let resolution = session.resolve(&written, &invalidation, context)?;
    stage(session, context, PublicationStage::RecheckFrameworkSources)?;
    session.recheck(&resolution, context)?;
    stage(session, context, PublicationStage::WriteMetadata)?;
    session.metadata(&captured, &resolution, context)?;
    context.check()?;
    session.finish(extraction, captured, written, invalidation, resolution)
}
#[derive(Debug, PartialEq, Eq)]
pub struct InvalidationPolicy {
    pub imports: bool,
    pub frameworks: bool,
    pub rust: bool,
    pub providers: bool,
}

impl InvalidationPolicy {
    pub fn for_publication(language: &str, config_changed: bool) -> Self {
        let modules =
            config_changed || matches!(language, "typescript" | "tsx" | "javascript" | "jsx");
        Self {
            imports: modules,
            frameworks: modules || language == "vue",
            rust: language == "rust",
            providers: modules,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_and_configuration_select_only_required_engines() {
        for language in ["typescript", "tsx", "javascript", "jsx"] {
            assert_eq!(
                InvalidationPolicy::for_publication(language, false),
                InvalidationPolicy {
                    imports: true,
                    frameworks: true,
                    rust: false,
                    providers: true,
                }
            );
        }
        assert_eq!(
            InvalidationPolicy::for_publication("vue", false),
            InvalidationPolicy {
                imports: false,
                frameworks: true,
                rust: false,
                providers: false,
            }
        );
        for language in ["csharp", "html", "rust"] {
            let policy = InvalidationPolicy::for_publication(language, false);
            assert!(!policy.imports && !policy.frameworks && !policy.providers);
            assert_eq!(policy.rust, language == "rust");
            let changed = InvalidationPolicy::for_publication(language, true);
            assert!(changed.imports && changed.frameworks && changed.providers);
        }
    }
}

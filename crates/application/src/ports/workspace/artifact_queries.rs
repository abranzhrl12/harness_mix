use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

/// Queries and transformations of structured artifacts.
pub trait ArtifactQueries: Send + Sync {
    fn svg(
        &self,
        _args: &harness_domain::svg::SvgArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "SVG adapter unavailable"))
    }
    fn data_query(
        &self,
        _args: &harness_domain::contracts::DataQueryArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Data query adapter unavailable"))
    }
}

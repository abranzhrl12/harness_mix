use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

/// Queries and transformations of structured artifacts.
pub trait ArtifactQueries: Send + Sync {
    fn word_preview(&self, _args: &harness_domain::word::WordPreviewArgs, _context: &OperationContext) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "Word preview adapter unavailable"))
    }
    fn xlsx_inspect(
        &self,
        _args: &harness_domain::xlsx::XlsxInspectArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "XLSX inspect adapter unavailable"))
    }
    fn xlsx_read(
        &self,
        _args: &harness_domain::xlsx::XlsxReadArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "XLSX read adapter unavailable"))
    }
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

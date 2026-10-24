use harness_domain::{
    Failure,
    browser::{BrowserArgs, BrowserResult},
    execution::OperationContext,
};

/// Browser inspection boundary. The adapter owns connection/runtime details.
pub trait BrowserInspection: Send + Sync {
    fn browser(
        &self,
        _args: &BrowserArgs,
        _context: &OperationContext,
    ) -> Result<BrowserResult, Failure> {
        Err(("UNSUPPORTED", "Browser inspection adapter unavailable"))
    }
}

use std::sync::Arc;

use harness_application::ports::browser::BrowserInspection;
use harness_domain::{
    Failure,
    browser::{BrowserArgs, BrowserResult},
    execution::OperationContext,
};

use crate::domain::{validate_connect_endpoint, validate_request};

/// Outbound port implemented by a concrete browser adapter.
pub trait BrowserBackend: Send + Sync {
    fn execute(
        &self,
        args: &BrowserArgs,
        context: &OperationContext,
    ) -> Result<BrowserResult, Failure>;
}

/// Browser use case: validate and honor cancellation before invoking its backend.
pub(crate) struct BrowserUseCase {
    backend: Arc<dyn BrowserBackend>,
}

impl BrowserUseCase {
    pub(crate) fn with_backend(backend: Arc<dyn BrowserBackend>) -> Self {
        Self { backend }
    }
}

impl BrowserInspection for BrowserUseCase {
    fn browser(
        &self,
        args: &BrowserArgs,
        context: &OperationContext,
    ) -> Result<BrowserResult, Failure> {
        context.check()?;
        validate_request(args)?;
        if let BrowserArgs::Connect { endpoint, .. } = args {
            validate_connect_endpoint(endpoint)?;
        }
        self.backend.execute(args, context)
    }
}

#[cfg(test)]
mod tests;

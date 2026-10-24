use std::sync::Arc;

use harness_application::ports::browser::BrowserInspection;
use harness_domain::{
    Failure,
    browser::{BrowserArgs, BrowserResult},
    execution::OperationContext,
};

use crate::{
    application::{BrowserBackend, BrowserUseCase},
    infrastructure::CdpBrowserBackend,
};

/// Default composition for the browser inspection use case.
pub struct BrowserService {
    use_case: BrowserUseCase,
}

impl BrowserService {
    /// Create the production service with its lazy CDP infrastructure adapter.
    pub fn new() -> Self {
        Self::with_backend(Arc::new(CdpBrowserBackend::new()))
    }

    /// Create the service with an injected backend, useful for alternative adapters and tests.
    pub fn with_backend(backend: Arc<dyn BrowserBackend>) -> Self {
        Self {
            use_case: BrowserUseCase::with_backend(backend),
        }
    }
}

impl Default for BrowserService {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserInspection for BrowserService {
    fn browser(
        &self,
        args: &BrowserArgs,
        context: &OperationContext,
    ) -> Result<BrowserResult, Failure> {
        self.use_case.browser(args, context)
    }
}

#[cfg(test)]
#[path = "composition_tests.rs"]
mod tests;

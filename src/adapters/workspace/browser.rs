//! Delegate browser I/O through the capability injected by the composition root.
use crate::adapters::NativeWorkspace;
use harness_application::ports::browser::BrowserInspection;
use harness_domain::{
    Failure,
    browser::{BrowserArgs, BrowserResult},
    execution::OperationContext,
};

impl BrowserInspection for NativeWorkspace {
    fn browser(
        &self,
        args: &BrowserArgs,
        context: &OperationContext,
    ) -> Result<BrowserResult, Failure> {
        self.browser.browser(args, context)
    }
}

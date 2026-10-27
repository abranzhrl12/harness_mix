use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use harness_application::ports::browser::BrowserInspection;
use harness_domain::{
    Failure,
    browser::{BrowserArgs, BrowserResult},
    execution::OperationContext,
};

use super::{BrowserBackend, BrowserUseCase};

struct FakeBackend {
    calls: AtomicUsize,
    fail: bool,
}

impl FakeBackend {
    fn succeeding() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            fail: false,
        }
    }

    fn failing() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            fail: true,
        }
    }
}

impl BrowserBackend for FakeBackend {
    fn execute(
        &self,
        _args: &BrowserArgs,
        _context: &OperationContext,
    ) -> Result<BrowserResult, Failure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            Err(("BACKEND_FAILURE", "backend failed"))
        } else {
            Ok(BrowserResult::Connected {
                connection_id: "injected".into(),
            })
        }
    }
}

fn connect(endpoint: &str) -> BrowserArgs {
    BrowserArgs::Connect {
        endpoint: endpoint.into(),
        session: Some("test-session".into()),
    }
}

#[test]
fn invalid_endpoint_is_rejected_before_backend_io() {
    let backend = Arc::new(FakeBackend::succeeding());
    let service = BrowserUseCase::with_backend(backend.clone());

    let result = service.browser(
        &connect("ws://example.com:9222/devtools/browser/id"),
        &OperationContext::without_deadline(),
    );

    assert_eq!(result.unwrap_err().0, "BROWSER_ENDPOINT");
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn cancelled_operation_is_rejected_before_backend_io() {
    let backend = Arc::new(FakeBackend::succeeding());
    let service = BrowserUseCase::with_backend(backend.clone());
    let context = OperationContext::without_deadline();
    context.cancel();

    let result = service.browser(
        &connect("ws://localhost:9222/devtools/browser/id"),
        &context,
    );

    assert_eq!(result.unwrap_err().0, "CANCELLED");
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn service_delegates_valid_requests_and_returns_backend_result() {
    let backend = Arc::new(FakeBackend::succeeding());
    let service = BrowserUseCase::with_backend(backend.clone());

    let result = service
        .browser(
            &connect("ws://localhost:9222/devtools/browser/id"),
            &OperationContext::without_deadline(),
        )
        .unwrap();

    assert!(matches!(
        result,
        BrowserResult::Connected { connection_id } if connection_id == "injected"
    ));
    assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn local_http_discovery_endpoint_reaches_backend() {
    let backend = Arc::new(FakeBackend::succeeding());
    let service = BrowserUseCase::with_backend(backend.clone());

    let result = service
        .browser(
            &connect("http://localhost:9222/json/version"),
            &OperationContext::without_deadline(),
        )
        .unwrap();

    assert!(matches!(
        result,
        BrowserResult::Connected { connection_id } if connection_id == "injected"
    ));
    assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn invalid_http_discovery_endpoint_is_rejected_before_backend_io() {
    let backend = Arc::new(FakeBackend::succeeding());
    let service = BrowserUseCase::with_backend(backend.clone());

    let result = service.browser(
        &connect("http://example.com:9222/json/version"),
        &OperationContext::without_deadline(),
    );

    assert_eq!(result.unwrap_err().0, "BROWSER_ENDPOINT");
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn service_preserves_backend_failures() {
    let backend = Arc::new(FakeBackend::failing());
    let service = BrowserUseCase::with_backend(backend.clone());

    let result = service.browser(
        &connect("ws://localhost:9222/devtools/browser/id"),
        &OperationContext::without_deadline(),
    );

    assert_eq!(result.unwrap_err(), ("BACKEND_FAILURE", "backend failed"));
    assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
}

use harness_application::{
    handlers::{edit, read, search, verify},
    ports::workspace::{SourceEditing, SourceQueries, Verification},
};
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};

struct QueryService;

impl SourceQueries for QueryService {
    fn read(
        &self,
        _: &harness_domain::contracts::ReadArgs,
        _: &OperationContext,
    ) -> Result<Value, Failure> {
        Ok(json!({"operation": "read"}))
    }

    fn search(
        &self,
        _: &harness_domain::contracts::SearchArgs,
        _: &OperationContext,
    ) -> Result<Value, Failure> {
        Ok(json!({"operation": "search"}))
    }

    fn explore(
        &self,
        _: &harness_domain::contracts::ExploreArgs,
        _: &OperationContext,
    ) -> Result<Value, Failure> {
        Ok(json!({"nodes": [], "edges": []}))
    }
}

struct EditingService;

impl SourceEditing for EditingService {
    fn replace(
        &self,
        _: &harness_domain::contracts::ReplaceArgs,
        _: &OperationContext,
    ) -> Result<Value, Failure> {
        Ok(json!({"changed": true}))
    }
}

struct VerificationService;

impl Verification for VerificationService {
    fn verify(
        &self,
        _: &harness_domain::contracts::VerifyArgs,
        _: &OperationContext,
    ) -> Result<Value, Failure> {
        Ok(json!({"passed": true}))
    }
}

macro_rules! args {
    ($value:expr) => {
        serde_json::from_value($value).expect("valid test arguments")
    };
}

#[test]
fn search_and_read_handlers_preserve_service_results() {
    let service = QueryService;
    let context = OperationContext::default();
    let search_args = args!(json!({"query": "needle"}));
    let read_args = args!(json!({"path": "src/lib.rs"}));

    assert_eq!(
        search::handle(&service, &search_args, &context).unwrap(),
        json!({"operation": "search"})
    );
    assert_eq!(
        read::read(&service, &read_args, &context).unwrap(),
        json!({"operation": "read"})
    );
}

#[test]
fn explore_handler_preserves_impact_projection_and_full_response() {
    let service = QueryService;
    let context = OperationContext::default();
    let impact_args = args!(json!({
        "filePath": "src/lib.rs",
        "symbol": "run",
        "view": "impact"
    }));
    let full_args = args!(json!({
        "filePath": "src/lib.rs",
        "symbol": "run",
        "view": "full"
    }));
    let raw = json!({"nodes": [], "edges": []});

    assert_eq!(
        read::explore(&service, &impact_args, &context).unwrap(),
        harness_domain::assistance::impact(&impact_args, raw.clone())
    );
    assert_eq!(read::explore(&service, &full_args, &context).unwrap(), raw);
}

#[test]
fn edit_handler_needs_only_the_editing_service_and_preserves_result() {
    let service = EditingService;
    let context = OperationContext::default();
    let args = harness_domain::contracts::ReplaceArgs::default();

    assert_eq!(
        edit::replace(&service, &args, &context).unwrap(),
        json!({"changed": true})
    );
}

#[test]
fn verify_handler_needs_only_the_verification_service_and_preserves_result() {
    let service = VerificationService;
    let context = OperationContext::default();
    let args = args!(json!({"language": "rust"}));

    assert_eq!(
        verify::handle(&service, &args, &context).unwrap(),
        json!({"passed": true})
    );
}

#[test]
fn handlers_preserve_port_failures_and_the_callers_context() {
    struct FailingVerification;
    impl Verification for FailingVerification {
        fn verify(
            &self,
            _: &harness_domain::contracts::VerifyArgs,
            context: &OperationContext,
        ) -> Result<Value, Failure> {
            context.report_progress("verification", 1, None, None);
            context.check()?;
            Err(("VERIFY_SCOPE", "scope rejected by the adapter"))
        }
    }

    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let progress = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&progress);
    let context = OperationContext::without_deadline().with_progress(move |_| {
        observed.fetch_add(1, Ordering::Relaxed);
    });
    let args = args!(json!({"language": "rust"}));
    assert_eq!(
        verify::handle(&FailingVerification, &args, &context).unwrap_err(),
        ("VERIFY_SCOPE", "scope rejected by the adapter")
    );
    assert_eq!(progress.load(Ordering::Relaxed), 1);
    context.cancel();
    assert_eq!(
        verify::handle(&FailingVerification, &args, &context)
            .unwrap_err()
            .0,
        "CANCELLED"
    );
}

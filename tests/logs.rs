use agent_harness::adapters::{self, NativeApplication};
use harness_domain::{
    contracts::{LogsArgs, ReadArgs, Request},
    execution::OperationContext,
};
use serde_json::Value;
fn logs(app: &NativeApplication, level: Option<&str>, after: Option<u64>) -> Value {
    app.execute(Request::Logs(LogsArgs {
        limit: Some(50),
        level: level.map(Into::into),
        tool: None,
        slow_only: None,
        request_id: None,
        after,
        session: None,
    }))
}
#[test]
fn logs_filter_worker_failures_without_paths_arguments_or_contents() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    app.execute(Request::Read(ReadArgs {
        path: "secret-request-path.txt".into(),
        start: 1,
        end: 1,
        session: None,
    }));
    let errors = logs(&app, Some("error"), None);
    assert_eq!(errors["state"], "ok", "{errors}");
    assert_eq!(errors["data"]["events"][0]["tool"], "harness_read");
    assert_eq!(errors["data"]["events"][0]["code"], "PATH");
    assert!(!errors["data"].to_string().contains("secret-request-path"));
    let cursor = errors["data"]["nextAfter"].as_u64().unwrap();
    let after = logs(&app, None, Some(cursor));
    assert_eq!(after["data"]["events"][0]["tool"], "harness_logs");
    assert_eq!(after["data"]["events"][0]["state"], "ok");
    let restarted = adapters::application(root.path()).unwrap();
    assert!(
        logs(&restarted, None, None)["data"]["events"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn cancelled_rejections_and_abandoned_work_are_warnings() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let request = || {
        Request::Read(ReadArgs {
            path: "x".into(),
            start: 1,
            end: 1,
            session: None,
        })
    };
    drop(
        app.prepare(request(), OperationContext::default())
            .unwrap_or_else(|_| panic!("fixture rejected")),
    );
    let cancelled = OperationContext::default();
    cancelled.cancel();
    assert!(app.prepare(request(), cancelled).is_err());
    let warnings = logs(&app, Some("warn"), None);
    assert_eq!(warnings["data"]["events"][0]["kind"], "abandoned");
    assert_eq!(warnings["data"]["events"][1]["kind"], "rejected");
    assert_eq!(warnings["data"]["events"][1]["code"], "CANCELLED");
    let unsupported = app.execute(Request::Logs(LogsArgs {
        limit: None,
        level: None,
        tool: None,
        slow_only: None,
        request_id: Some("x".into()),
        after: None,
        session: None,
    }));
    assert_eq!(unsupported["diagnostics"][0]["code"], "ARGUMENT");
}

#[test]
fn request_ids_correlate_completed_rejected_and_abandoned_operations() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let request = || {
        Request::Read(ReadArgs {
            path: "x".into(),
            start: 1,
            end: 1,
            session: None,
        })
    };
    let completed = app.execute(request());
    let cancelled = OperationContext::default();
    cancelled.cancel();
    let rejected = app.prepare(request(), cancelled).err().unwrap();
    let pending = app
        .prepare(request(), OperationContext::default())
        .unwrap_or_else(|_| panic!("fixture rejected"));
    let abandoned = pending.request_id().to_owned();
    drop(pending);
    let ids = [
        completed["requestId"].as_str().unwrap(),
        rejected["requestId"].as_str().unwrap(),
        &abandoned,
    ];
    for (id, kind) in ids.into_iter().zip(["completed", "rejected", "abandoned"]) {
        assert!(harness_domain::execution::valid_request_id(id));
        let result = app.execute(Request::Logs(LogsArgs {
            limit: None,
            level: None,
            tool: None,
            slow_only: None,
            request_id: Some(id.into()),
            after: None,
            session: None,
        }));
        assert_eq!(result["state"], "ok", "{result}");
        assert_eq!(result["data"]["events"].as_array().unwrap().len(), 1);
        assert_eq!(result["data"]["events"][0]["requestId"], id);
        assert_eq!(result["data"]["events"][0]["kind"], kind);
    }
    assert_ne!(ids[0], ids[1]);
    assert_ne!(ids[1], ids[2]);
}

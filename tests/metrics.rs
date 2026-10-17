use agent_harness::adapters::{self, NativeApplication};
use harness_domain::{
    contracts::{BootstrapArgs, CheckpointArgs, DetailArgs, DoctorArgs, ReadArgs, Request},
    execution::OperationContext,
};
use serde_json::Value;

fn request() -> Request {
    Request::Read(ReadArgs {
        path: "private-path-not-retained.txt".into(),
        start: 1,
        end: 1,
        session: None,
    })
}
fn snapshot(app: &NativeApplication) -> Value {
    let result = app.execute(Request::Doctor(DoctorArgs {
        versions: Some(false),
        connection: Some(false),
        session: None,
    }));
    assert_eq!(result["state"], "ok", "{result}");
    result["data"]["metrics"].clone()
}

#[test]
fn clones_share_metrics_but_restarts_and_sessions_do_not_restore_counters() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("good.txt"), "fixture").unwrap();
    let app = adapters::application(root.path()).unwrap();
    assert_eq!(app.execute(request())["state"], "error");
    let clone = app.clone();
    assert_eq!(
        clone.execute(Request::Read(ReadArgs {
            path: "good.txt".into(),
            start: 1,
            end: 1,
            session: None
        }))["state"],
        "ok"
    );
    let metrics = snapshot(&clone);
    assert_eq!(metrics["totals"]["attempted"], 3);
    assert_eq!(metrics["totals"]["completed"], 2);
    assert_eq!(metrics["totals"]["active"], 1); // The doctor query itself.
    assert_eq!(metrics["byTool"]["harness_read"]["outcomes"]["ok"], 1);
    assert_eq!(metrics["byTool"]["harness_read"]["outcomes"]["error"], 1);
    assert!(!metrics.to_string().contains("private-path-not-retained"));
    let boot = app.execute(Request::Bootstrap(BootstrapArgs {
        session: None,
        include_rules: None,
    }));
    let session = boot["data"]["sessionId"].as_str().unwrap().to_owned();
    let checkpoint = app.execute(Request::Checkpoint(CheckpointArgs {
        session: session.clone(),
        goal: None,
        decisions: None,
        pending: None,
        refresh_freshness: None,
    }));
    assert_eq!(checkpoint["data"]["stats"]["available"], true);
    assert_eq!(checkpoint["data"]["stats"]["scope"], "application_instance");
    assert_eq!(checkpoint["data"]["stats"]["sessionSpecific"], false);
    let detail = app.execute(Request::Detail(DetailArgs {
        reference: checkpoint["detail"]["ref"].as_str().unwrap().into(),
        offset: None,
        max_chars: Some(16000),
        session: None,
    }));
    let full: Value = serde_json::from_str(detail["data"]["text"].as_str().unwrap()).unwrap();
    assert_eq!(full["stats"], checkpoint["data"]["stats"]);
    let stored = std::fs::read_to_string(
        root.path()
            .join(format!(".agents/harness/.state/{session}.json")),
    )
    .unwrap();
    assert!(!stored.contains("byTool"));
    let restarted = adapters::application(root.path()).unwrap();
    let metrics = snapshot(&restarted);
    assert_eq!(metrics["totals"]["attempted"], 1);
    assert_eq!(metrics["totals"]["completed"], 0);
}

#[test]
fn rejection_abandonment_and_worker_cancellation_are_counted_once() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let mut held = Vec::new();
    for _ in 0..4 {
        held.push(
            app.prepare(request(), OperationContext::default())
                .unwrap_or_else(|_| panic!("fixture admission rejected")),
        );
    }
    let rejected = app
        .prepare(request(), OperationContext::default())
        .err()
        .unwrap();
    assert_eq!(rejected["diagnostics"][0]["code"], "QUEUE_FULL");
    drop(held.pop());
    let metrics = snapshot(&app);
    assert_eq!(metrics["totals"]["attempted"], 6);
    assert_eq!(metrics["totals"]["admitted"], 5);
    assert_eq!(metrics["totals"]["active"], 4);
    assert_eq!(metrics["totals"]["abandoned"], 1);
    assert_eq!(metrics["totals"]["rejected"], 1);
    assert_eq!(metrics["totals"]["rejections"]["queueFull"], 1);
    assert_eq!(metrics["totals"]["peakActive"], 4);
    drop(held);
    let cancelled = OperationContext::default();
    cancelled.cancel();
    assert!(app.prepare(request(), cancelled).is_err());
    let cancelled = OperationContext::default();
    let worker = app
        .prepare(request(), cancelled.clone())
        .unwrap_or_else(|_| panic!("fixture admission rejected"));
    cancelled.cancel();
    assert_eq!(worker.run()["state"], "cancelled");
    assert!(
        app.prepare(
            request(),
            OperationContext::with_timeout(std::time::Duration::ZERO)
        )
        .is_err()
    );
    let metrics = snapshot(&app);
    assert_eq!(metrics["totals"]["abandoned"], 4);
    assert_eq!(metrics["totals"]["rejections"]["cancelled"], 1);
    assert_eq!(metrics["totals"]["rejections"]["timeout"], 1);
    assert_eq!(
        metrics["byTool"]["harness_read"]["outcomes"]["cancelled"],
        1
    );
    let totals = &metrics["totals"];
    assert_eq!(
        totals["admitted"].as_u64().unwrap(),
        totals["completed"].as_u64().unwrap()
            + totals["abandoned"].as_u64().unwrap()
            + totals["active"].as_u64().unwrap()
    );
}

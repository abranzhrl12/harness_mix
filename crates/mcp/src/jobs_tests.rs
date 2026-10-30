use super::*;
use harness_application::ports::{JobEventFuture, JobEventStream, JobFuture};
use serde_json::json;
use std::sync::{Arc, Mutex};

struct FakeJobs(Arc<Mutex<Option<TaskArgs>>>);

impl JobService for FakeJobs {
    fn handle(&self, args: TaskArgs) -> JobFuture<'_> {
        *self.0.lock().unwrap() = Some(args);
        Box::pin(async { Ok(json!({"accepted": true})) })
    }

    fn subscribe(&self) -> Box<dyn JobEventStream> {
        Box::new(FakeEvents)
    }
}

struct FakeEvents;
impl JobEventStream for FakeEvents {
    fn recv(&mut self) -> JobEventFuture<'_> {
        Box::pin(async { harness_application::ports::JobEvent::Closed })
    }
}

#[tokio::test]
async fn jobs_transport_dispatches_through_application_port() {
    let seen = Arc::new(Mutex::new(None));
    let fake = FakeJobs(seen.clone());
    let args = TaskArgs {
        action: harness_domain::tasks::TaskAction::List,
        task_id: None,
        after: None,
        verify: None,
        timeout_seconds: None,
        max_attempts: None,
        dedupe: false,
        filter: None,
        lines: None,
        before: None,
        stream: None,
        cursor: None,
        max_chars: None,
    };

    let result = handle(Some(&fake), args, false).await;
    assert_eq!(seen.lock().unwrap().as_ref().unwrap().action, harness_domain::tasks::TaskAction::List);
    assert_eq!(result.structured_content.as_ref().unwrap()["state"], "ok");
}

#[tokio::test]
async fn unavailable_jobs_keep_profile_error_response() {
    let result = handle(None, task_args(), false).await;
    assert_eq!(
        result.structured_content.as_ref().unwrap()["state"],
        "error"
    );
    assert_eq!(
        result.structured_content.as_ref().unwrap()["diagnostics"][0]["code"],
        "TOOL_PROFILE"
    );
}

#[test]
fn event_mapping_preserves_progress_and_lag_recovery() {
    let event = json!({"taskId":"t1","kind":"output","data":{"stream":"stdout","text":"working"}});
    assert_eq!(
        super::event_value(harness_application::ports::JobEvent::Event(event.clone())),
        Some(event)
    );
    assert_eq!(
        super::event_value(harness_application::ports::JobEvent::Lagged(3)),
        Some(json!({"kind":"events_lagged","skipped":3,"recovery":"harness_jobs action=events"}))
    );
    assert_eq!(
        super::event_value(harness_application::ports::JobEvent::Closed),
        None
    );
}

fn task_args() -> TaskArgs {
    TaskArgs {
        action: harness_domain::tasks::TaskAction::List,
        task_id: None,
        after: None,
        verify: None,
        timeout_seconds: None,
        max_attempts: None,
        dedupe: false,
        filter: None,
        lines: None,
        before: None,
        stream: None,
        cursor: None,
        max_chars: None,
    }
}

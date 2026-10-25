use harness_application::ports::JobService;
use harness_domain::{results::Results, tasks::TaskArgs};
use rmcp::model::CallToolResult;

pub(super) fn event_value(
    event: harness_application::ports::JobEvent,
) -> Option<serde_json::Value> {
    match event {
        harness_application::ports::JobEvent::Event(value) => Some(value),
        harness_application::ports::JobEvent::Lagged(skipped) => Some(
            serde_json::json!({"kind":"events_lagged","skipped":skipped,"recovery":"harness_jobs action=events"}),
        ),
        harness_application::ports::JobEvent::Closed => None,
    }
}

pub(super) async fn handle(
    service: Option<&dyn JobService>,
    args: TaskArgs,
    context_presentation: bool,
) -> CallToolResult {
    let arguments = serde_json::to_value(&args).expect("task arguments");
    let result = match service {
        Some(service) => service.handle(args).await,
        None => Err((
            "TOOL_PROFILE",
            "Background jobs are unavailable in this tool profile",
        )),
    };
    super::presentation::response_with_options(
        Results::default().envelope("harness_jobs", &arguments, result),
        false,
        context_presentation,
    )
}

#[cfg(test)]
#[path = "jobs_tests.rs"]
mod tests;

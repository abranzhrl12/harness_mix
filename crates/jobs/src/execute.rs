use super::Inner;
use harness_domain::{
    Failure, contracts::VerifyArgs, execution::OperationContext, jobs::JobStatus,
    process::StopReason,
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio::sync::Notify;

pub(super) fn retryable(code: &str, attempt: u8, maximum: u8) -> bool {
    code == "PROCESS_SPAWN_TRANSIENT" && attempt < maximum
}

pub(super) async fn execute(
    inner: Arc<Inner>,
    id: String,
    verify_args: VerifyArgs,
    timeout: Option<Duration>,
    context: OperationContext,
    wake: Arc<Notify>,
    attempts: u8,
) {
    let permit = tokio::select! {
        permit = inner.permits.clone().acquire_owned() => permit.ok(),
        _ = wake.notified() => None,
    };
    if permit.is_none() || context.check().is_err() {
        inner.update(&id, JobStatus::Cancelled, 0, None);
        return;
    }
    for attempt in 1..=attempts {
        if context.check().is_err() {
            inner.update(&id, JobStatus::Cancelled, attempt - 1, None);
            return;
        }
        let prepared = match inner.executor.prepare(&inner.root, &verify_args, timeout) {
            Ok(prepared) => prepared,
            Err((code, message)) => {
                inner.update(
                    &id,
                    JobStatus::Failed,
                    attempt,
                    Some(json!({"error":code,"message":message})),
                );
                return;
            }
        };
        if !inner.update(&id, JobStatus::Running, attempt, None) {
            return;
        }
        let work_inner = inner.clone();
        let work_id = id.clone();
        let execution_timeout =
            timeout.or_else(|| verify_args.timeout_seconds.map(Duration::from_secs));
        let work_context = context.with_execution_timeout(execution_timeout);
        let result = tokio::task::spawn_blocking(move || {
            let outcome = prepared.execute(&work_context, &mut |stderr, bytes| {
                work_inner.output(&work_id, stderr, bytes)
            })?;
            let status = match outcome.reason {
                StopReason::Cancelled => JobStatus::Cancelled,
                StopReason::TimedOut => JobStatus::TimedOut,
                StopReason::Exited if outcome.exit_code == Some(0) => JobStatus::Succeeded,
                _ => JobStatus::Failed,
            };
            let output = format!(
                "{}\n{}",
                String::from_utf8_lossy(&outcome.stdout),
                String::from_utf8_lossy(&outcome.stderr)
            );
            let mut value = json!({"passed":status == JobStatus::Succeeded,"code":outcome.exit_code,"milliseconds":outcome.elapsed.as_millis() as u64,"output":output.chars().take(12000).collect::<String>(),"outputTruncated":output.chars().count()>12000,"limited":outcome.reason == StopReason::OutputLimit,"treeTerminationConfirmed":outcome.tree_termination_confirmed,"verificationFreshness":"not_tracked"});
            prepared.annotate(&outcome, &mut value);
            Ok::<_, Failure>((status, value))
        })
        .await;
        match result {
            Ok(Ok((status, result))) => {
                let status = if status == JobStatus::Succeeded && result["passed"] != true {
                    JobStatus::Failed
                } else {
                    status
                };
                inner.update(&id, status, attempt, Some(result));
                return;
            }
            Ok(Err((code, _))) if retryable(code, attempt, attempts) => {
                if !inner.update(&id, JobStatus::RetryWait, attempt, None) {
                    return;
                }
                tokio::select! { _ = tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))) => {}, _ = wake.notified() => {} }
            }
            Ok(Err((code, message))) => {
                inner.update(
                    &id,
                    if code == "TIMEOUT" {
                        JobStatus::TimedOut
                    } else if code == "CANCELLED" || context.check().is_err() {
                        JobStatus::Cancelled
                    } else {
                        JobStatus::Failed
                    },
                    attempt,
                    Some(json!({"error":code,"message":message})),
                );
                return;
            }
            Err(_) => {
                inner.update(
                    &id,
                    JobStatus::Failed,
                    attempt,
                    Some(json!({"error":"TASK_WORKER","message":"Background worker failed"})),
                );
                return;
            }
        }
    }
}

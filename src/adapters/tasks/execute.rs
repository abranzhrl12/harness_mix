use super::{Inner, verify};
use harness_domain::{
    contracts::VerifyArgs, execution::OperationContext, jobs::JobStatus, process::StopReason,
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
    let verify_args = Arc::new(verify_args);
    let permit = tokio::select! {
        permit = inner.permits.clone().acquire_owned() => permit.ok(),
        _ = wake.notified() => None,
    };
    if permit.is_none() || context.check().is_err() {
        inner.update(&id, JobStatus::Cancelled, 0, None);
        return;
    }
    let verify_args = Arc::new(verify_args);
    for attempt in 1..=attempts {
        if context.check().is_err() {
            inner.update(&id, JobStatus::Cancelled, attempt - 1, None);
            return;
        }
        let spec = match verify::background_plan(&inner.root, &verify_args, timeout) {
            Ok(spec) => spec,
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
        let work_context = context.with_execution_timeout(timeout);
        let work_args = verify_args.clone();
        let syntax_only = matches!(spec, verify::BackgroundVerificationPlan::TypeScriptSyntax);
        let result = tokio::task::spawn_blocking(move || match spec {
            verify::BackgroundVerificationPlan::Process(spec) => (work_inner.executor)(
                &work_inner.root,
                &spec,
                &work_context,
                &mut |stderr, bytes| work_inner.output(&work_id, stderr, bytes),
            ),
            verify::BackgroundVerificationPlan::TypeScriptSyntax => {
                let outcome =
                    verify::run_typescript_syntax(&work_inner.root, &work_args, &work_context)?;
                for chunk in outcome.stdout.chunks(8192) {
                    work_inner.output(&work_id, false, chunk);
                }
                for chunk in outcome.stderr.chunks(8192) {
                    work_inner.output(&work_id, true, chunk);
                }
                Ok(outcome)
            }
        })
        .await;
        match result {
            Ok(Ok(outcome)) => {
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
                let mut result = json!({"passed":status == JobStatus::Succeeded,"code":outcome.exit_code,"milliseconds":outcome.elapsed.as_millis() as u64,"output":output.chars().take(12000).collect::<String>(),"outputTruncated":output.chars().count()>12000,"limited":outcome.reason == StopReason::OutputLimit,"treeTerminationConfirmed":outcome.tree_termination_confirmed,"verificationFreshness":"not_tracked"});
                if syntax_only {
                    result["language"] = json!("typescript");
                    result["verificationMode"] = json!("typescript_syntax_only");
                    result["typeCheckPerformed"] = json!(false);
                    result["codeExecuted"] = json!(false);
                    result["testCountVerified"] = json!(false);
                }
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

//! Native process status, TCP probing and HTTP loopback health checking adapter.
//! Safe, bounded diagnostic tool; no shell, no global termination.

mod http_probe;
mod pid;
mod tcp_probe;
mod validation;

#[cfg(test)]
mod tests;

use harness_domain::{Failure, contracts::ProcessStatusArgs, execution::OperationContext};
use serde_json::{Value, json};
use std::time::Instant;

pub fn handle_process_status(
    args: &ProcessStatusArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    handle_process_status_with_tasks(args, context, None)
}

pub fn handle_process_status_with_tasks(
    args: &ProcessStatusArgs,
    context: &OperationContext,
    tasks: Option<&super::tasks::TaskManager>,
) -> Result<Value, Failure> {
    context.check()?;
    let options = validation::validate(args)?;
    let started = Instant::now();

    let mut result = json!({
        "sampledAt": chrono::Utc::now().to_rfc3339(),
    });

    if let Some(pid) = args.pid {
        result["process"] = pid::check_pid(pid);
    }

    if args.run_id.is_some() || args.list.is_some() {
        let jobs = tasks
            .ok_or(("UNSUPPORTED", "Managed job registry is not attached"))?
            .process_status(args)?;
        for (key, value) in jobs
            .as_object()
            .ok_or(("INTERNAL", "Invalid job status result"))?
        {
            result[key] = value.clone();
        }
    }

    if let Some(port) = args.port {
        let host = args.host.as_deref().unwrap_or("127.0.0.1");
        context.check()?;
        result["tcp"] = tcp_probe::probe(host, port, options.timeout);
    }

    if let Some(url) = &options.health_url {
        context.check()?;
        result["health"] = http_probe::probe(url, options.timeout);
    }

    result["milliseconds"] = json!(started.elapsed().as_millis() as u64);
    result["limitations"] = json!(
        "Point-in-time observations. PID existence does not prove responsiveness or identity (PID reuse). TCP acceptance is not application health or ownership by the requested PID. HTTP health means a 2xx status only, not body validation. Run IDs refer to retained harness_jobs task IDs; history survives restart and interrupted work is not rerun."
    );

    Ok(result)
}

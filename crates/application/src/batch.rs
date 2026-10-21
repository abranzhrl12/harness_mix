use crate::{
    Application,
    ports::{SessionRepository, Workspace},
};
use harness_domain::{
    Failure, contracts::BatchArgs, execution::OperationContext, results::utf16_len,
};
use serde_json::{Value, json};

impl<W: Workspace, S: SessionRepository> Application<W, S> {
    pub(super) fn batch(
        &self,
        args: BatchArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        if !(1..=4).contains(&args.jobs.len()) {
            return Err(("ARGUMENT", "Batch requires 1..4 independent read jobs"));
        }
        let max_concurrent = args.max_concurrent.unwrap_or(2);
        if !(2..=4).contains(&max_concurrent) {
            return Err(("ARGUMENT", "maxConcurrent must be between 2 and 4"));
        }
        // Decode every child before execution so callers see all input errors at once.
        let mut jobs = Vec::with_capacity(args.jobs.len());
        let mut validation_errors = Vec::new();
        for (index, job) in args.jobs.into_iter().enumerate() {
            let tool = job.tool.name();
            match job.into_request_detailed(args.session.as_deref()) {
                Ok(request) if request.is_mutating() => {
                    let field = (tool == "harness_explore").then_some("syncIndex");
                    let reason = "batch child must be read-only";
                    validation_errors.push(json!({
                        "code": "ARGUMENT",
                        "index": index,
                        "tool": tool,
                        "field": field,
                        "reason": reason,
                        "message": format!("Batch child {index} ({tool}) must be read-only"),
                    }));
                }
                Ok(request) => jobs.push((index, tool, request)),
                Err(error) => {
                    let message = format!(
                        "Batch child {index} ({tool}) has invalid arguments: {}",
                        error.reason
                    );
                    validation_errors.push(json!({
                        "code": "ARGUMENT",
                        "index": index,
                        "tool": tool,
                        "field": error.field,
                        "reason": error.reason,
                        "message": message,
                    }));
                }
            }
        }
        if !validation_errors.is_empty() {
            return Ok(json!({
                "ok": false,
                "atomic": false,
                "results": [],
                "diagnostics": validation_errors,
            }));
        }
        let mut results: Vec<Option<Value>> =
            std::iter::repeat_with(|| None).take(jobs.len()).collect();
        let mut jobs = jobs.into_iter();
        std::thread::scope(|scope| {
            let (sender, receiver) = std::sync::mpsc::channel();
            let mut active = 0;
            let mut completed = 0;

            while completed < results.len() {
                while active < max_concurrent {
                    let Some((index, tool, request)) = jobs.next() else {
                        break;
                    };
                    let operation = match self.prepare(request, context.clone()) {
                        Ok(operation) => operation,
                        Err(result) => {
                            results[index] = Some(json!({"index":index,"tool":tool,"result":result}));
                            completed += 1;
                            continue;
                        }
                    };
                    let request_id = operation.request_id().to_owned();
                    let failure_request_id = request_id.clone();
                    let worker_sender = sender.clone();
                    match std::thread::Builder::new().spawn_scoped(scope, move || {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            operation.run()
                        }))
                        .map_err(|_| ());
                        let _ = worker_sender.send((index, tool, request_id, result));
                    }) {
                        Ok(_) => active += 1,
                        Err(_) => {
                            let mut result = self.failure(
                                tool,
                                &json!({}),
                                ("INTERNAL", "Batch child worker could not start"),
                            );
                            result["requestId"] = json!(failure_request_id);
                            results[index] = Some(json!({"index":index,"tool":tool,"result":result}));
                            completed += 1;
                        }
                    }
                }

                if active == 0 {
                    continue;
                }
                let (index, tool, request_id, result) = receiver
                    .recv()
                    .expect("active batch workers always report completion");
                active -= 1;
                completed += 1;
                let mut result = result.unwrap_or_else(|_| {
                    self.failure(
                        tool,
                        &json!({}),
                        ("INTERNAL", "Batch child worker failed"),
                    )
                });
                result["requestId"] = json!(request_id);
                results[index] = Some(json!({"index":index,"tool":tool,"result":result}));
            }
        });
        let results: Vec<_> = results.into_iter().flatten().collect();
        let ok = results.iter().all(|item| item["result"]["state"] == "ok");
        let compact:Vec<_> = results.iter().map(|item| {
            let result = &item["result"];
            json!({"index":item["index"],"tool":item["tool"],"state":result["state"],
                "requestId":result["requestId"],"data":result["data"],"diagnostics":result["diagnostics"],"omitted":result["omitted"],"detail":result["detail"]})
        }).collect();
        let mut output = json!({"ok":ok,"results":compact,"atomic":false,"maxConcurrentChildren":max_concurrent});
        if utf16_len(&output.to_string()) > 20000 {
            output["results"] = json!(results.iter().map(|item|json!({"index":item["index"],"tool":item["tool"],"requestId":item["result"]["requestId"],"state":item["result"]["state"],"fullResultInBatchDetail":true})).collect::<Vec<_>>());
            output["truncated"] = json!(true);
            output["_detail"] = json!({"results":results,"atomic":false});
        }
        if !ok {
            output["diagnostics"] = json!([{"code":"BATCH_PARTIAL","message":"One or more read jobs did not succeed; inspect results"}]);
        }
        if let Err((code, _)) = context.check() {
            output["cancelled"] = json!(code == "CANCELLED");
            output["timedOut"] = json!(code == "TIMEOUT");
            output["_complete"] = json!(false);
        }
        Ok(output)
    }
}

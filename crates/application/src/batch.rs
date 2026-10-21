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
        let mut results = Vec::new();
        let mut jobs = jobs.into_iter();
        loop {
            let pair: Vec<_> = jobs.by_ref().take(max_concurrent).collect();
            if pair.is_empty() {
                break;
            }
            std::thread::scope(|scope| {
                let workers: Vec<_> = pair
                    .into_iter()
                    .map(|(index, tool, request)| {
                        let name = tool;
                        let (request_id, handle) = match self.prepare(request, context.clone()) {
                            Err(output) => (None, Err(output)),
                            Ok(operation) => {
                                let id = operation.request_id().to_owned();
                                let worker = std::thread::Builder::new()
                                    .spawn_scoped(scope, move || operation.run())
                                    .map_err(|_| {
                                        self.failure(
                                            name,
                                            &json!({}),
                                            ("INTERNAL", "Batch child worker could not start"),
                                        )
                                    });
                                (Some(id), worker)
                            }
                        };
                        (index, name, request_id, handle)
                    })
                    .collect();
                for (index, name, request_id, worker) in workers {
                    let mut result = match worker {
                        Ok(worker) => worker.join().unwrap_or_else(|_| {
                            self.failure(
                                name,
                                &json!({}),
                                ("INTERNAL", "Batch child worker failed"),
                            )
                        }),
                        Err(output) => output,
                    };
                    if let Some(id) = request_id {
                        result["requestId"] = json!(id);
                    }
                    results.push(json!({"index":index,"tool":name,"result":result}));
                }
            });
        }
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

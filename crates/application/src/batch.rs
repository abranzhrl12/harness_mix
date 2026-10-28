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
                            results[index] =
                                Some(json!({"index":index,"tool":tool,"result":result}));
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
                            results[index] =
                                Some(json!({"index":index,"tool":tool,"result":result}));
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
                    self.failure(tool, &json!({}), ("INTERNAL", "Batch child worker failed"))
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
                "requestId":result["requestId"],"scope":result["scope"],"data":result["data"],"diagnostics":result["diagnostics"],"omitted":result["omitted"],"detail":result["detail"]})
        }).collect();
        let mut output = json!({"ok":ok,"results":compact,"atomic":false,"maxConcurrentChildren":max_concurrent});
        if utf16_len(&output.to_string()) > 20000 {
            output["results"] = json!(
                results
                    .iter()
                    .map(|item| {
                        let result = &item["result"];
                        let (data, preview_truncated) =
                            batch_preview(item["tool"].as_str().unwrap_or(""), &result["data"]);
                        let mut child = json!({"index":item["index"],"tool":item["tool"],
                    "requestId":result["requestId"],"scope":result["scope"],"state":result["state"],
                    "data":data,"diagnostics":result["diagnostics"],
                    "omitted":result["omitted"],"detail":result["detail"]});
                        if preview_truncated {
                            child["data"]["previewTruncated"] = json!(true);
                        }
                        child
                    })
                    .collect::<Vec<_>>()
            );
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

/// Keep a useful, tool-shaped preview for each child when the batch envelope
/// exceeds its response budget. The original child results stay in `_detail`.
fn batch_preview(tool: &str, data: &Value) -> (Value, bool) {
    let Some(source) = data.as_object() else {
        return (data.clone(), false);
    };
    let mut preview = serde_json::Map::new();
    let mut shortened = false;
    let copy = |key: &str, preview: &mut serde_json::Map<String, Value>| {
        if let Some(value) = source.get(key) {
            preview.insert(key.to_owned(), value.clone());
        }
    };
    for key in [
        "path",
        "hash",
        "language",
        "syntaxOnly",
        "notice",
        "warnings",
        "complete",
        "truncated",
        "totalLines",
        "rowCount",
        "engine",
        "kind",
        "resolvedName",
        "definitionView",
        "indexFreshness",
        "fileFreshness",
        "omitted",
        "filtered",
        "symbol",
    ] {
        copy(key, &mut preview);
    }
    match tool {
        "harness_read" => {
            for key in ["start", "end", "range"] {
                copy(key, &mut preview);
            }
            if let Some(text) = source.get("text").and_then(Value::as_str) {
                let (excerpt, cut) = source_excerpt(text, 2800);
                preview.insert("text".into(), json!(excerpt));
                shortened |= cut;
            }
        }
        "harness_search" => {
            for key in ["paths", "files", "matches"] {
                if let Some(values) = source.get(key).and_then(Value::as_array) {
                    let cap = match key {
                        "paths" => 16,
                        "files" => 16,
                        _ => 6,
                    };
                    let mut out = Vec::new();
                    for value in values.iter().take(cap) {
                        let mut row = value.clone();
                        if key == "paths" {
                            if let Some(path) = row.as_str() {
                                let (path, cut) = short_text(path, 160);
                                shortened |= cut;
                                row = json!(path);
                            }
                        } else if key == "files" {
                            if let Some(object) = row.as_object_mut() {
                                if let Some(path) = object.get("path").and_then(Value::as_str) {
                                    let (path, cut) = short_text(path, 160);
                                    object.insert("path".into(), json!(path));
                                    shortened |= cut;
                                }
                            }
                        } else {
                            if let Some(object) = row.as_object_mut() {
                                if let Some(text) = object.get("text").and_then(Value::as_str) {
                                    let (excerpt, cut) = short_text(text, 160);
                                    object.insert("text".into(), json!(excerpt));
                                    shortened |= cut;
                                }
                                for key in ["signature", "doc"] {
                                    if let Some(text) = object.get(key).and_then(Value::as_str) {
                                        let (excerpt, cut) = short_text(text, 200);
                                        object.insert(key.into(), json!(excerpt));
                                        shortened |= cut;
                                    }
                                }
                                if let Some(context) =
                                    object.get_mut("context").and_then(Value::as_object_mut)
                                {
                                    if let Some(text) = context.get("text").and_then(Value::as_str)
                                    {
                                        let (excerpt, cut) = source_excerpt(text, 300);
                                        context.insert("text".into(), json!(excerpt));
                                        shortened |= cut;
                                    }
                                }
                            }
                        }
                        out.push(row);
                    }
                    shortened |= values.len() > cap;
                    preview.insert(key.to_owned(), json!(out));
                }
            }
            for key in ["count", "total", "selection", "scanComplete", "coverage"] {
                copy(key, &mut preview);
            }
        }
        "harness_symbols" => {
            if let Some(text) = source.get("text").and_then(Value::as_str) {
                let (excerpt, cut) = source_excerpt(text, 2800);
                preview.insert("text".into(), json!(excerpt));
                shortened |= cut;
            }
            if let Some(items) = source.get("items").and_then(Value::as_array) {
                let mut shown = Vec::new();
                for item in items.iter().take(12) {
                    let mut item = item.clone();
                    if let Some(object) = item.as_object_mut() {
                        if let Some(signature) = object.get("signature").and_then(Value::as_str) {
                            let (value, cut) = short_text(signature, 160);
                            object.insert("signature".into(), json!(value));
                            shortened |= cut;
                        }
                    }
                    shown.push(item);
                }
                shortened |= items.len() > 12;
                preview.insert("items".into(), json!(shown));
            }
            copy("selection", &mut preview);
        }
        "harness_sql" => {
            for key in ["truncated", "parametersComplete", "definition"] {
                copy(key, &mut preview);
            }
            for key in ["columns", "parameters"] {
                if let Some(items) = source.get(key).and_then(Value::as_array) {
                    let mut shown = Vec::new();
                    for item in items.iter().take(32) {
                        let mut item = item.clone();
                        if let Some(object) = item.as_object_mut() {
                            for field in ["column", "name", "type", "typeName"] {
                                if let Some(text) = object.get(field).and_then(Value::as_str) {
                                    let (excerpt, cut) = short_text(text, 120);
                                    object.insert(field.into(), json!(excerpt));
                                    shortened |= cut;
                                }
                            }
                        }
                        shown.push(item);
                    }
                    shortened |= items.len() > 32;
                    preview.insert(key.into(), json!(shown));
                }
            }
            if let Some(rows) = source.get("rows").and_then(Value::as_array) {
                let mut shown = Vec::new();
                for row in rows.iter().take(4) {
                    let mut row = row.clone();
                    if let Some(values) = row.as_object_mut() {
                        let mut remaining = 8usize;
                        let before = values.len();
                        values.retain(|_, _| {
                            let keep = remaining > 0;
                            remaining = remaining.saturating_sub(1);
                            keep
                        });
                        shortened |= before > values.len();
                        for value in values.values_mut() {
                            if let Some(text) = value.as_str() {
                                let (excerpt, cut) = short_text(text, 96);
                                *value = json!(excerpt);
                                shortened |= cut;
                            }
                        }
                    }
                    shown.push(row);
                }
                shortened |= rows.len() > 4;
                preview.insert("rows".into(), json!(shown));
            }
            if let Some(definition) = source.get("definition").and_then(Value::as_str) {
                let (excerpt, cut) = source_excerpt(definition, 3000);
                preview.insert("definition".into(), json!(excerpt));
                shortened |= cut;
            }
        }
        "harness_git" => {
            for key in [
                "branch",
                "head",
                "upstream",
                "comparison",
                "changed",
                "totalLines",
            ] {
                copy(key, &mut preview);
            }
            if let Some(text) = source.get("preview").and_then(Value::as_str) {
                let (excerpt, cut) = source_excerpt(text, 3000);
                preview.insert("preview".into(), json!(excerpt));
                shortened |= cut;
            }
            if let Some(entries) = source.get("entries").and_then(Value::as_array) {
                shortened |= entries.len() > 64;
                preview.insert(
                    "entries".into(),
                    json!(entries.iter().take(64).collect::<Vec<_>>()),
                );
            }
        }
        _ => {
            // Generic fallback retains scalar metadata and only shortens large
            // source-like strings; unknown nested contracts remain recoverable.
            for (key, value) in source {
                if preview.contains_key(key) {
                    continue;
                }
                if let Some(text) = value.as_str() {
                    let (excerpt, cut) = short_text(text, 1200);
                    preview.insert(key.clone(), json!(excerpt));
                    shortened |= cut;
                }
            }
        }
    }
    (Value::Object(preview), shortened)
}

fn short_text(text: &str, max_chars: usize) -> (String, bool) {
    if text.chars().count() <= max_chars {
        return (text.to_owned(), false);
    }
    (
        text.chars()
            .take(max_chars.saturating_sub(1))
            .collect::<String>()
            + "…",
        true,
    )
}

fn source_excerpt(text: &str, max_chars: usize) -> (String, bool) {
    if text.chars().count() <= max_chars {
        return (text.to_owned(), false);
    }
    let prefix = text.chars().take(max_chars).collect::<String>();
    let end = prefix.rfind('\n').unwrap_or(prefix.len());
    let mut excerpt = prefix[..end].to_owned();
    if !excerpt.is_empty() {
        excerpt.push_str("\n…");
    }
    (excerpt, true)
}

#[cfg(test)]
mod tests {
    use super::batch_preview;
    use serde_json::json;

    #[test]
    fn oversized_batch_previews_keep_source_and_hash_with_a_line_boundary() {
        let source = format!("{}\nlast line", "let x = 1;\n".repeat(500));
        let (preview, shortened) = batch_preview(
            "harness_read",
            &json!({
                "path":"src/a.rs","hash":"full-sha256","start":7,"end":506,
                "text":source,"totalLines":500
            }),
        );
        assert!(shortened);
        assert_eq!(preview["path"], "src/a.rs");
        assert_eq!(preview["hash"], "full-sha256");
        let text = preview["text"].as_str().unwrap();
        assert!(text.ends_with("\n…"));
        assert!(
            text.lines()
                .take_while(|line| *line != "…")
                .all(|line| line == "let x = 1;")
        );
    }

    #[test]
    fn oversized_search_previews_preserve_full_hash_and_context_range() {
        let long = "match".repeat(200);
        let (preview, shortened) = batch_preview(
            "harness_search",
            &json!({
                "matches":[{"file":"src/a.rs","line":3,"hash":"full-digest",
                    "text":long,"context":{"start":1,"end":5,"text":"a\nb\nc\nd\ne"}}]
            }),
        );
        assert!(shortened);
        assert_eq!(preview["matches"][0]["hash"], "full-digest");
        assert_eq!(preview["matches"][0]["context"]["start"], 1);
    }
}

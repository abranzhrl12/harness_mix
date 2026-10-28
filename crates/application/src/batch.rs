use crate::{
    ports::{SessionRepository, Workspace},
    Application,
};
use harness_domain::{
    contracts::{BatchArgs, Request},
    execution::OperationContext,
    results::utf16_len,
    Failure,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;

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
                Ok(request) => {
                    let target = batch_job_target(tool, &request);
                    jobs.push((index, tool, target, request));
                }
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
                    let Some((index, tool, target, request)) = jobs.next() else {
                        break;
                    };
                    let operation = match self.prepare(request, context.clone()) {
                        Ok(operation) => operation,
                        Err(result) => {
                            let mut item = json!({"index":index,"tool":tool,"result":result});
                            if let Some(target) = target {
                                item["target"] = json!(target);
                            }
                            results[index] = Some(item);
                            completed += 1;
                            continue;
                        }
                    };
                    let request_id = operation.request_id().to_owned();
                    let failure_request_id = request_id.clone();
                    let worker_sender = sender.clone();
                    let worker_target = target.clone();
                    match std::thread::Builder::new().spawn_scoped(scope, move || {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            operation.run()
                        }))
                        .map_err(|_| ());
                        let _ =
                            worker_sender.send((index, tool, worker_target, request_id, result));
                    }) {
                        Ok(_) => active += 1,
                        Err(_) => {
                            let mut result = self.failure(
                                tool,
                                &json!({}),
                                ("INTERNAL", "Batch child worker could not start"),
                            );
                            result["requestId"] = json!(failure_request_id);
                            let mut item = json!({"index":index,"tool":tool,"result":result});
                            if let Some(target) = target {
                                item["target"] = json!(target);
                            }
                            results[index] = Some(item);
                            completed += 1;
                        }
                    }
                }

                if active == 0 {
                    continue;
                }
                let (index, tool, target, request_id, result) = receiver
                    .recv()
                    .expect("active batch workers always report completion");
                active -= 1;
                completed += 1;
                let mut result = result.unwrap_or_else(|_| {
                    self.failure(tool, &json!({}), ("INTERNAL", "Batch child worker failed"))
                });
                result["requestId"] = json!(request_id);
                let mut item = json!({"index":index,"tool":tool,"result":result});
                if let Some(target) = target {
                    item["target"] = json!(target);
                }
                results[index] = Some(item);
            }
        });
        let results: Vec<_> = results.into_iter().flatten().collect();
        let ok = results.iter().all(|item| item["result"]["state"] == "ok");
        let compact: Vec<_> = results
            .iter()
            .map(|item| {
                let result = &item["result"];
                let mut child = json!({
                    "index":item["index"],
                    "tool":item["tool"],
                    "state":result["state"],
                    "requestId":result["requestId"],
                    "scope":result["scope"],
                    "data":result["data"],
                    "diagnostics":result["diagnostics"],
                    "omitted":result["omitted"],
                    "detail":result["detail"],
                });
                if let Some(target) = item.get("target").filter(|v| v.is_string()) {
                    child["target"] = target.clone();
                }
                child
            })
            .collect();
        let mut output = json!({"ok":ok,"results":compact,"atomic":false,"maxConcurrentChildren":max_concurrent});
        let child_exceeds_budget = results
            .iter()
            .any(|item| utf16_len(&item["result"]["data"].to_string()) > 12000);
        if utf16_len(&output.to_string()) > 48000 || child_exceeds_budget {
            let mut any_truncated = false;
            output["results"] = json!(results
                .iter()
                .map(|item| {
                    let result = &item["result"];
                    let (data, preview_truncated) =
                        batch_preview(item["tool"].as_str().unwrap_or(""), &result["data"]);
                    any_truncated |= preview_truncated;
                    let mut child = json!({
                        "index":item["index"],
                        "tool":item["tool"],
                        "requestId":result["requestId"],
                        "scope":result["scope"],
                        "state":result["state"],
                        "data":data,
                        "diagnostics":result["diagnostics"],
                        "omitted":result["omitted"],
                        "detail":result["detail"],
                    });
                    if let Some(target) = item.get("target").filter(|v| v.is_string()) {
                        child["target"] = target.clone();
                    }
                    if preview_truncated {
                        child["data"]["previewTruncated"] = json!(true);
                    }
                    child
                })
                .collect::<Vec<_>>());
            if any_truncated || utf16_len(&output.to_string()) > 48000 {
                output["truncated"] = json!(true);
                output["_detail"] = json!({"results":results,"atomic":false});
            }
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

fn batch_job_target(_tool: &str, request: &Request) -> Option<String> {
    let raw = match request {
        Request::Read(args) => format!("{}:{}-{}", args.path, args.start, args.end),
        Request::Search(args) => {
            if let Some(cursor) = &args.cursor {
                format!(
                    "cursor:{}@{}",
                    short_text(cursor, 24).0,
                    args.offset.unwrap_or(0)
                )
            } else {
                let kind = match args.kind {
                    harness_domain::contracts::Kind::Text => "text",
                    harness_domain::contracts::Kind::Files => "files",
                    harness_domain::contracts::Kind::FileSize => "file_size",
                    harness_domain::contracts::Kind::Syntax => "syntax",
                };
                match (args.query.trim(), args.scope.as_deref()) {
                    ("", Some(scope)) => format!("{kind} in {scope}"),
                    ("", None) => kind.to_owned(),
                    (q, Some(scope)) => format!("{q} in {scope}"),
                    (q, None) => q.to_owned(),
                }
            }
        }
        Request::Outline(args) => match args.query.as_deref() {
            Some(q) if !q.is_empty() => format!("outline {} ({q})", args.path),
            _ => format!("outline {}", args.path),
        },
        Request::ReadSymbol(args) => format!("{}::{}", args.path, args.symbol),
        Request::VueExtractPreview(args) => format!("{}::{}", args.path, args.symbol_id),
        Request::Explore(args) => {
            let mut parts = Vec::new();
            if let Some(fp) = args.file_path.as_deref().filter(|s| !s.is_empty()) {
                parts.push(fp.to_owned());
            }
            if let Some(sym) = args.symbol.as_deref().filter(|s| !s.is_empty()) {
                parts.push(sym.to_owned());
            } else if let Some(id) = args.symbol_id.as_deref().filter(|s| !s.is_empty()) {
                parts.push(id.to_owned());
            } else if !args.query.trim().is_empty() {
                parts.push(args.query.trim().to_owned());
            }
            if parts.is_empty() {
                "explore".to_owned()
            } else {
                parts.join("::")
            }
        }
        Request::GitStatus(_) => "status".to_owned(),
        Request::GitDiff(args) => format!("diff {}", args.path),
        Request::SqlStatus(_) => "status".to_owned(),
        Request::SqlSearch(args) => format!("search {}", args.query),
        Request::SqlMap(args) => match args.schema.as_deref() {
            Some(schema) if !schema.is_empty() => format!("map {schema}"),
            _ => "map".to_owned(),
        },
        Request::SqlDescribe(args) => format!("describe {}", args.name),
        Request::SqlQuery(args) => short_text(args.sql.trim(), 80).0,
        Request::Config(args) => format!("{}#{}", args.path, args.section),
        Request::Detail(args) => format!(
            "ref:{}@{}",
            short_text(&args.reference, 24).0,
            args.offset.unwrap_or(0)
        ),
        Request::XlsxInspect(args) => args.path.clone(),
        Request::XlsxRead(args) => format!("{}#{}", args.path, args.sheet),
        Request::Svg(args) => args.path.clone(),
        Request::Verify(args) => match args.scope.as_deref() {
            Some(scope) if !scope.is_empty() => format!("{} ({scope})", args.language),
            _ => args.language.clone(),
        },
        _ => return None,
    };
    Some(short_text(&raw, 120).0)
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
        "query",
        "scope",
        "selector",
        "hasMore",
        "nextOffset",
        "nextCall",
        "detailRef",
    ] {
        copy(key, &mut preview);
    }
    match tool {
        "harness_read" => {
            for key in ["start", "end", "range", "nextStart"] {
                copy(key, &mut preview);
            }
            if let Some(text) = source.get("text").and_then(Value::as_str) {
                let (excerpt, cut) = source_excerpt(text, 8000);
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
                    if key == "matches" {
                        let omitted_matches = source
                            .get("omitted")
                            .and_then(|v| v.as_u64().or_else(|| v["matches"].as_u64()))
                            .unwrap_or(0) as usize;
                        let total_matches = source
                            .get("totalMatches")
                            .or_else(|| source.get("total"))
                            .or_else(|| source.get("count"))
                            .and_then(Value::as_u64)
                            .unwrap_or((values.len() + omitted_matches) as u64);
                        preview.insert("totalMatches".into(), json!(total_matches));
                        let matched_files: BTreeSet<&str> = values
                            .iter()
                            .filter_map(|row| row["file"].as_str().or_else(|| row["path"].as_str()))
                            .collect();
                        if !matched_files.is_empty() {
                            preview.insert("matchedFiles".into(), json!(matched_files.len()));
                        }
                    } else if key == "paths" || key == "files" {
                        let total = source
                            .get("count")
                            .or_else(|| source.get("total"))
                            .and_then(Value::as_u64)
                            .unwrap_or(values.len() as u64);
                        preview.insert("totalMatches".into(), json!(total));
                        preview.insert("matchedFiles".into(), json!(values.len()));
                    }
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
                        } else if let Some(object) = row.as_object_mut() {
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
                                if let Some(text) = context.get("text").and_then(Value::as_str) {
                                    let (excerpt, cut) = source_excerpt(text, 300);
                                    context.insert("text".into(), json!(excerpt));
                                    shortened |= cut;
                                }
                            }
                        }
                        out.push(row);
                    }
                    shortened |= values.len() > cap;
                    preview.insert(key.to_owned(), json!(out));
                }
            }
            for key in [
                "count",
                "total",
                "selection",
                "scanComplete",
                "coverage",
                "minLines",
                "retrieval",
            ] {
                copy(key, &mut preview);
            }
        }
        "harness_symbols" => {
            if let Some(text) = source.get("text").and_then(Value::as_str) {
                let (excerpt, cut) = source_excerpt(text, 8000);
                preview.insert("text".into(), json!(excerpt));
                shortened |= cut;
            }
            if let Some(items) = source.get("items").and_then(Value::as_array) {
                preview.insert("totalItems".into(), json!(items.len()));
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
            for key in ["selection", "inventory", "candidates"] {
                copy(key, &mut preview);
            }
        }
        "harness_explore" => {
            for key in [
                "view",
                "file",
                "focus",
                "files",
                "counts",
                "relationshipKinds",
                "locationKind",
                "ambiguous",
                "candidatesOmitted",
                "callersOmitted",
                "selection",
                "contextSelection",
                "contextTruncation",
                "synchronized",
                "absenceProven",
                "editReady",
                "traversal",
                "truncation",
            ] {
                copy(key, &mut preview);
            }
            if let Some(text) = source.get("text").and_then(Value::as_str) {
                let (excerpt, cut) = source_excerpt(text, 6000);
                preview.insert("text".into(), json!(excerpt));
                shortened |= cut;
            }
            if let Some(definitions) = source.get("definitions").and_then(Value::as_array) {
                preview.insert("totalDefinitions".into(), json!(definitions.len()));
                let mut shown = Vec::new();
                for def in definitions.iter().take(8) {
                    let mut def = def.clone();
                    if let Some(sym) = def.get_mut("symbol").and_then(Value::as_object_mut) {
                        if let Some(sig) = sym.get("signature").and_then(Value::as_str) {
                            let (excerpt, cut) = short_text(sig, 200);
                            sym.insert("signature".into(), json!(excerpt));
                            shortened |= cut;
                        }
                    }
                    shown.push(def);
                }
                shortened |= definitions.len() > 8;
                preview.insert("definitions".into(), json!(shown));
            }
            if let Some(callers) = source.get("callers").and_then(Value::as_array) {
                let shown: Vec<_> = callers.iter().take(10).cloned().collect();
                shortened |= callers.len() > 10;
                preview.insert("callers".into(), json!(shown));
            }
            for key in ["unresolved", "candidates"] {
                if let Some(items) = source.get(key).and_then(Value::as_array) {
                    let shown: Vec<_> = items.iter().take(8).cloned().collect();
                    shortened |= items.len() > 8;
                    preview.insert(key.into(), json!(shown));
                }
            }
            if let Some(graph) = source.get("graph").and_then(Value::as_object) {
                let mut g = serde_json::Map::new();
                for key in [
                    "targetIds",
                    "traversal",
                    "truncation",
                    "relationsOmitted",
                    "truncated",
                ] {
                    if let Some(v) = graph.get(key) {
                        g.insert(key.into(), v.clone());
                    }
                }
                if let Some(nodes) = graph.get("nodes").and_then(Value::as_array) {
                    g.insert("nodeCount".into(), json!(nodes.len()));
                    let shown: Vec<_> = nodes.iter().take(12).cloned().collect();
                    shortened |= nodes.len() > 12;
                    g.insert("nodes".into(), json!(shown));
                }
                if let Some(edges) = graph.get("edges").and_then(Value::as_array) {
                    g.insert("edgeCount".into(), json!(edges.len()));
                    let shown: Vec<_> = edges.iter().take(12).cloned().collect();
                    shortened |= edges.len() > 12;
                    g.insert("edges".into(), json!(shown));
                }
                if let Some(relations) = graph.get("relations").and_then(Value::as_array) {
                    g.insert("relationCount".into(), json!(relations.len()));
                    let shown: Vec<_> = relations.iter().take(12).cloned().collect();
                    shortened |= relations.len() > 12;
                    g.insert("relations".into(), json!(shown));
                }
                preview.insert("graph".into(), Value::Object(g));
            }
        }
        "harness_sql" => {
            for key in [
                "truncated",
                "parametersComplete",
                "definition",
                "status",
                "configured",
                "readOnly",
                "name",
                "schema",
            ] {
                copy(key, &mut preview);
            }
            for key in ["columns", "parameters", "items", "objects", "tables"] {
                if let Some(items) = source.get(key).and_then(Value::as_array) {
                    let mut shown = Vec::new();
                    for item in items.iter().take(32) {
                        let mut item = item.clone();
                        if let Some(object) = item.as_object_mut() {
                            for field in ["column", "name", "type", "typeName", "schema"] {
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
                "ahead",
                "behind",
                "comparison",
                "changed",
                "totalLines",
                "counts",
                "files",
                "note",
            ] {
                copy(key, &mut preview);
            }
            if let Some(text) = source.get("preview").and_then(Value::as_str) {
                let (excerpt, cut) = source_excerpt(text, 3000);
                preview.insert("preview".into(), json!(excerpt));
                shortened |= cut;
            }
            if let Some(entries) = source.get("entries").and_then(Value::as_array) {
                preview.insert("totalEntries".into(), json!(entries.len()));
                shortened |= entries.len() > 64;
                preview.insert(
                    "entries".into(),
                    json!(entries.iter().take(64).collect::<Vec<_>>()),
                );
            }
        }
        "harness_verify" => {
            for key in [
                "status",
                "action",
                "scope",
                "package",
                "project",
                "exitCode",
                "counts",
                "errorCount",
                "warningCount",
                "summary",
            ] {
                copy(key, &mut preview);
            }
            for key in ["errors", "warnings", "items", "diagnostics"] {
                if let Some(items) = source.get(key).and_then(Value::as_array) {
                    let shown: Vec<_> = items.iter().take(8).cloned().collect();
                    shortened |= items.len() > 8;
                    preview.insert(key.into(), json!(shown));
                }
            }
            for key in ["stdout", "stderr", "output"] {
                if let Some(text) = source.get(key).and_then(Value::as_str) {
                    let (excerpt, cut) = source_excerpt(text, 2000);
                    preview.insert(key.into(), json!(excerpt));
                    shortened |= cut;
                }
            }
        }
        _ => {
            // Generic fallback retains scalar metadata and bounded array previews,
            // only shortening large source-like strings; full nested contracts remain recoverable.
            for (key, value) in source {
                if preview.contains_key(key) {
                    continue;
                }
                match value {
                    Value::String(text) => {
                        let (excerpt, cut) = short_text(text, 1200);
                        preview.insert(key.clone(), json!(excerpt));
                        shortened |= cut;
                    }
                    Value::Bool(_) | Value::Number(_) | Value::Null => {
                        preview.insert(key.clone(), value.clone());
                    }
                    Value::Array(items) => {
                        let shown: Vec<_> = items.iter().take(8).cloned().collect();
                        shortened |= items.len() > 8;
                        preview.insert(key.clone(), json!(shown));
                    }
                    Value::Object(map) if map.len() <= 8 => {
                        preview.insert(key.clone(), value.clone());
                    }
                    Value::Object(_) => {
                        shortened = true;
                    }
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
        let source = format!("{}\nlast line", "let x = 1;\n".repeat(1000));
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
        assert!(text
            .lines()
            .take_while(|line| *line != "…")
            .all(|line| line == "let x = 1;"));
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
        assert_eq!(preview["totalMatches"], 1);
        assert_eq!(preview["matchedFiles"], 1);
    }

    #[test]
    fn oversized_explore_previews_keep_counts_definitions_and_bounded_graph() {
        let defs: Vec<_> = (0..12)
            .map(|i| {
                json!({
                    "path": format!("src/f{i}.rs"),
                    "role": if i == 0 { "target" } else { "related" },
                    "symbol": {"name": format!("sym_{i}"), "signature": "fn sym()"}
                })
            })
            .collect();
        let (preview, shortened) = batch_preview(
            "harness_explore",
            &json!({
                "view": "full",
                "text": "fn sym() {}\n".repeat(800),
                "definitions": defs,
                "graph": {
                    "nodes": [{"id": "n1"}, {"id": "n2"}],
                    "edges": [{"source": "n1", "target": "n2", "kind": "calls"}],
                    "relations": [{"source": "sym_0", "target": "sym_1", "kind": "calls"}]
                }
            }),
        );
        assert!(shortened);
        assert_eq!(preview["totalDefinitions"], 12);
        assert_eq!(preview["definitions"].as_array().unwrap().len(), 8);
        assert_eq!(preview["graph"]["nodeCount"], 2);
        assert_eq!(preview["graph"]["edgeCount"], 1);
        assert_eq!(preview["graph"]["relationCount"], 1);
    }
}

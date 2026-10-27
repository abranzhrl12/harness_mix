//! Readable batch presentation; canonical envelopes remain untouched.
use serde_json::Value;

pub(super) fn with_notices(mut text: String, envelope: &Value) -> String {
    let data = &envelope["data"];
    for diagnostics in [&envelope["diagnostics"], &data["warnings"]] {
        if let Some(items) = diagnostics.as_array() {
            for item in items {
                let code = item["code"].as_str().unwrap_or("NOTICE");
                let message = item.as_str().or_else(|| item["message"].as_str()).unwrap_or("");
                text.push_str(&format!("\n> {}: {}\n", super::clip(code, 80), super::clip(message, 300)));
            }
        }
    }
    let flags = ["truncated", "presentationTruncated", "contentTruncated", "hasMore", "snapshotOnly"];
    let partial = flags.iter().any(|key| data[*key] == true)
        || data["scanComplete"] == false
        || data["complete"] == false
        || data.get("contextTruncation").is_some()
        || envelope.get("omitted").is_some_and(|value| match value {
            Value::Object(map) => !map.is_empty(),
            Value::Number(number) => number.as_u64().is_some_and(|n| n > 0),
            _ => false,
        });
    if partial {
        text.push_str("\n> Partial output; check structuredContent for limits, omissions and continuation.\n");
    }
    if let Some(reference) = envelope["detail"]["ref"].as_str() {
        text.push_str(&format!("\n> detail.ref: {}\n", super::clip(reference, 100)));
    }
    text
}

pub(super) fn render(envelope: &Value) -> Option<String> {
    render_inner(envelope, 0)
}

fn render_inner(envelope: &Value, depth: usize) -> Option<String> {
    let children = envelope["data"]["results"].as_array()?;
    let failed = children.iter().filter(|child| child["state"] != "ok").count();
    let mut text = format!("## Batch · {} results · {failed} failed\n", children.len());
    for (index, child) in children.iter().enumerate() {
        // Domain batches carry tool at the child root, rather than scope.tool.
        let tool = child["tool"].as_str()
            .or_else(|| child["scope"]["tool"].as_str()).unwrap_or("result");
        let state = child["state"].as_str().unwrap_or("unknown");
        text.push_str(&format!("\n---\n\n### {} · {} · {}\n\n",
            index + 1, super::clip(tool, 80), super::clip(state, 40)));
        let body = if state == "ok" {
            if tool == "harness_batch" && depth < 4 {
                render_inner(child, depth + 1)
            } else if tool == "harness_explore" && child["data"]["view"] == "full" {
                child["data"]["text"].as_str().map(str::to_owned)
            } else {
                super::lean_channel(tool, child)
            }
        } else {
            None
        };
        if let Some(body) = body {
            text.push_str(&with_notices(body, child));
        } else {
            // Preserve unfamiliar payloads and errors instead of guessing their meaning.
            // Pretty JSON is a readable fallback, never a JSON-escaped string.
            let body = serde_json::to_string_pretty(child).ok()?;
            let fence = super::fence_run(&body);
            text.push_str(&format!("{fence}json\n{body}\n{fence}\n"));
        }
    }
    Some(with_notices(text, envelope))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    fn wire(envelope: Value) -> Value {
        serde_json::to_value(super::super::response_with_options(envelope, false, false)).unwrap()
    }
    use super::*;

    #[test]
    fn batch_renders_children_as_source_and_preserves_canonical_envelopes() {
        let wire = wire(json!({
            "state":"ok","scope":{"tool":"harness_batch"},
            "data":{"results":[
                {"index":0,"tool":"harness_read","state":"ok","data":{
                    "path":"app.cs","start":9,"end":10,"text":"return x;\nreturn y;"}},
                {"index":1,"tool":"harness_symbols","state":"ok","data":{
                    "path":"app.js","symbol":{"name":"load","start":2,"end":2},"text":"fetch('/x');"}}
            ]}
        }));
        let text = wire["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("## Batch · 2 results · 0 failed"), "{text}");
        assert!(text.contains("```csharp\n9 | return x;\n10 | return y;"), "{text}");
        assert!(text.contains("```javascript\n2 | fetch('/x');"), "{text}");
        assert!(!text.contains("\\n"), "{text}");
        assert_eq!(wire["structuredContent"]["data"]["results"][0]["data"]["text"], "return x;\nreturn y;");
        assert_ne!(wire["isError"], true);
    }

    #[test]
    fn batch_retains_failed_child_diagnostics_and_recovery() {
        let envelope = json!({
            "state":"ok","scope":{"tool":"harness_batch"},"detail":{"ref":"batch-saved"},
            "data":{"results":[{"index":0,"tool":"harness_read","state":"error",
                "diagnostics":[{"code":"PATH","message":"missing file"}],
                "detail":{"ref":"child-saved"},"data":{}}]}
        });
        let wire = wire(envelope);
        let text = wire["content"][0]["text"].as_str().unwrap();
        for expected in ["1 failed", "harness_read · error", "PATH", "missing file", "child-saved", "batch-saved"] {
            assert!(text.contains(expected), "{text}");
        }
        assert_eq!(wire["structuredContent"]["data"]["results"][0]["state"], "error");
        assert_ne!(wire["isError"], true);
    }

    #[test]
    fn source_notices_keep_warnings_truncation_and_detail_visible() {
        let wire = wire(json!({
            "state":"ok","scope":{"tool":"harness_read"},
            "detail":{"ref":"saved"},"omitted":{"lines":4},
            "data":{"path":"a.rs","start":7,"end":7,"text":"let x = 1;",
                "warnings":[{"code":"STALE","message":"source changed"}]}
        }));
        let text = wire["content"][0]["text"].as_str().unwrap();
        for expected in ["STALE: source changed", "Partial output", "detail.ref: saved"] {
            assert!(text.contains(expected), "{text}");
        }
        assert_eq!(wire["structuredContent"]["omitted"]["lines"], 4);
    }
}

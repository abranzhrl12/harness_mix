use serde_json::json;

fn wire(envelope: serde_json::Value) -> serde_json::Value {
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
    assert!(
        text.contains("```csharp\n9 | return x;\n10 | return y;"),
        "{text}"
    );
    assert!(text.contains("```javascript\n2 | fetch('/x');"), "{text}");
    assert!(!text.contains("\\n"), "{text}");
    assert_eq!(
        wire["structuredContent"]["data"]["results"][0]["data"]["text"],
        "return x;\nreturn y;"
    );
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
    for expected in [
        "1 failed",
        "harness_read · error",
        "PATH",
        "missing file",
        "child-saved",
        "batch-saved",
    ] {
        assert!(text.contains(expected), "{text}");
    }
    assert_eq!(
        wire["structuredContent"]["data"]["results"][0]["state"],
        "error"
    );
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
    for expected in [
        "STALE: source changed",
        "Partial output",
        "detail.ref: saved",
    ] {
        assert!(text.contains(expected), "{text}");
    }
    assert_eq!(wire["structuredContent"]["omitted"]["lines"], 4);
}

#[test]
fn repeated_notices_are_deduplicated_and_batch_previews_explain_recovery() {
    let envelope = json!({
        "scope":{"tool":"harness_batch"},
        "detail":{"ref":"batch-full"},
        "omitted":{"truncated":true},
        "diagnostics":[{"code":"BATCH_PARTIAL","message":"inspect results"}],
        "data":{"warnings":[{"code":"BATCH_PARTIAL","message":"inspect results"}]}
    });
    let text = with_notices("Batch summary".into(), &envelope);
    assert_eq!(
        text.matches("BATCH_PARTIAL: inspect results").count(),
        1,
        "{text}"
    );
    assert!(
        text.contains("full results are recoverable from the batch detail"),
        "{text}"
    );
    assert!(text.contains("detail.ref: batch-full"), "{text}");

    let child = json!({"data":{"previewTruncated":true}});
    let child_text = with_notices("Child source".into(), &child);
    assert!(
        child_text.contains("full child results are recoverable"),
        "{child_text}"
    );
}

use super::{response_with_compact, response_with_presentation};
use serde_json::{Value, json};

fn sample() -> Value {
    json!({"state":"ok","requestId":"r1","scope":{"tool":"harness_explore"},
        "diagnostics":[],"omitted":{"truncated":true},"detail":{"ref":"saved","complete":false},
        "data":{"view":"full","text":"**Exploration: main**\n1\\tfn main() {}",
            "definitions":[{"path":"a.rs","hash":"hash","symbol":{"name":"main","symbolId":"local-id"},
                "role":"target","sourceStatus":"partial","nextCall":{"tool":"harness_symbols","arguments":{"action":"read","path":"a.rs","symbol":"main"}}}],
            "graph":{"nodes":[{"id":"one"}],"edges":[],"traversal":{"direction":"both","maxDepth":1,"limits":{"edges":160}},
                "truncation":{"depthBoundary":true}},"contextTruncation":{"textPreview":true},
            "unresolved":[],"indexFreshness":"not_certified","fileFreshness":{"status":"stale"},
            "synchronized":false,"absenceProven":false,"notice":"bounded, not exhaustive","reuseGuidance":"read missing context only"}})
}

#[test]
fn context_text_preserves_structured_graph_and_safety_metadata() {
    let envelope = sample();
    let result = response_with_presentation(envelope.clone(), true);
    assert_eq!(result.structured_content.as_ref(), Some(&envelope));
    let result = serde_json::to_value(result).unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("## Graph context"));
    assert!(text.contains("fn main() {}"));
    assert!(text.find("```json").unwrap() < text.find("fn main()").unwrap());
    let status = text
        .split("```json\n")
        .nth(1)
        .unwrap()
        .split("\n```")
        .next()
        .unwrap();
    let status: Value = serde_json::from_str(status).unwrap();
    assert_eq!(status["fileFreshness"]["status"], "stale");
    assert_eq!(status["contextTruncation"]["textPreview"], true);
    assert_eq!(status["coverage"][0]["nextCall"]["tool"], "harness_symbols");
    assert_eq!(status["detail"]["ref"], "saved");
    assert_eq!(status["requestId"], "r1");
    assert_eq!(status["editReady"], false);
}

#[test]
fn legacy_errors_and_summary_keep_their_json_content() {
    for (state, view, context) in [
        ("ok", "full", false),
        ("error", "full", true),
        ("ok", "summary", true),
    ] {
        let mut envelope = sample();
        envelope["state"] = json!(state);
        envelope["data"]["view"] = json!(view);
        let result =
            serde_json::to_value(response_with_presentation(envelope.clone(), context)).unwrap();
        let decoded: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(decoded, envelope);
        assert_eq!(result["isError"], state != "ok");
    }
}

#[test]
fn compact_removes_only_empty_envelope_metadata() {
    let envelope = json!({
        "state":"ok", "requestId":"internal-correlation-id", "scope":{"tool":"harness_read"},
        "diagnostics":[], "omitted":{}, "detail":null,
        "data":{"path":"file.rs", "hash":"sha256", "text":"exact source"}
    });
    let result = response_with_compact(envelope, true);
    let compact = result.structured_content.unwrap();
    assert_eq!(compact["state"], "ok");
    assert_eq!(compact["scope"]["tool"], "harness_read");
    assert_eq!(compact["data"]["text"], "exact source");
    assert!(compact.get("requestId").is_none());
    assert!(compact.get("diagnostics").is_none());
    assert!(compact.get("omitted").is_none());
    assert!(compact.get("detail").is_none());
}

#[test]
fn compact_preserves_nonempty_warnings_omissions_and_detail_refs() {
    let mut envelope = json!({
        "state":"ok", "requestId":"internal-correlation-id", "scope":{"tool":"harness_read"},
        "diagnostics":[{"code":"READ_WARNING"}], "omitted":{"text":true},
        "detail":{"ref":"detail-ref", "complete":false}, "data":{"text":"source"}
    });
    let expected_diagnostics = envelope["diagnostics"].clone();
    let expected_omitted = envelope["omitted"].clone();
    let expected_detail = envelope["detail"].clone();
    let compact = response_with_compact(envelope.take(), true)
        .structured_content.unwrap();
    assert_eq!(compact["diagnostics"], expected_diagnostics);
    assert_eq!(compact["omitted"], expected_omitted);
    assert_eq!(compact["detail"], expected_detail);
    assert!(compact.get("requestId").is_none());
}

#[test]
fn default_response_preserves_legacy_envelope() {
    let envelope = json!({
        "state":"ok", "requestId":"r1", "scope":{"tool":"harness_read"},
        "diagnostics":[], "omitted":{}, "detail":null, "data":{"text":"source"}
    });
    let result = response_with_presentation(envelope.clone(), false);
    assert_eq!(result.structured_content.as_ref(), Some(&envelope));
}

#[test]
fn default_compact_response_cleans_empty_diagnostics_omitted_and_detail() {
    let envelope = json!({
        "state":"ok", "requestId":"r1", "scope":{"tool":"harness_symbols"},
        "diagnostics":[], "omitted":{}, "detail":null, "data":{"found":true}
    });
    let result = response_with_compact(envelope, false);
    let structured = result.structured_content.unwrap();
    assert_eq!(structured["state"], "ok");
    assert_eq!(structured["scope"]["tool"], "harness_symbols");
    assert_eq!(structured["data"]["found"], true);
    assert_eq!(structured["requestId"], "r1");
    assert!(structured.get("diagnostics").is_none());
    assert!(structured.get("omitted").is_none());
    assert!(structured.get("detail").is_none());
}


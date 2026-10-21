use super::response_with_presentation;
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

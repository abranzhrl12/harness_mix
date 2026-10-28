use super::presentation::normalize_text_only_error;
use super::{response_with_compact, response_with_presentation};
use serde_json::{json, Value};

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
    // The readable channel carries a compact meta line, never duplicated JSON;
    // the structured envelope holds the full graph.
    assert!(
        !text.contains("```json"),
        "lean channel must not embed JSON: {text}"
    );
    assert!(text.contains("state ok"));
    assert!(text.contains("freshness index=not_certified file=stale"));
    assert!(text.contains("coverage: 1 targets + 0 related"));
    assert!(text.contains("graph: 1 nodes / 0 edges"));
    assert!(text.contains("unresolved: 0"));
    assert!(text.contains("detail.ref saved"));
    assert!(
        text.contains("TRUNCATED"),
        "truncation surfaces in the meta line: {text}"
    );
    assert!(text.contains("bounded, not exhaustive"));
    // Natural reading order: title and source never move behind relations.
    assert!(text.find("**Exploration: main**").unwrap() < text.find("fn main() {}").unwrap());
}

#[test]
fn context_text_lists_unresolved_symbols_in_the_meta_line() {
    let mut envelope = sample();
    envelope["data"]["unresolved"] = json!([{"symbol":"g","role":"related"}]);
    let result = serde_json::to_value(response_with_presentation(envelope, true)).unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("unresolved: 1"), "{text}");
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
        .structured_content
        .unwrap();
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
fn parameter_deserialization_errors_gain_structured_diagnostics_without_losing_text() {
    use rmcp::model::{CallToolResponse, CallToolResult, ContentBlock};

    let content = vec![ContentBlock::text(
        "Failed to deserialize parameters: unknown field `externalScopeId`",
    )];
    let original_content = serde_json::to_value(&content).unwrap();
    let response = CallToolResponse::Complete(CallToolResult::error(content));
    let normalized = normalize_text_only_error(response, "harness_search");
    let CallToolResponse::Complete(result) = normalized else {
        panic!("result response remains complete");
    };

    let envelope = result.structured_content.as_ref().unwrap();
    assert_eq!(envelope["state"], "error");
    assert_eq!(envelope["scope"]["tool"], "harness_search");
    assert_eq!(envelope["diagnostics"][0]["code"], "ARGUMENT");
    assert_eq!(
        envelope["diagnostics"][0]["message"],
        "Failed to deserialize parameters: unknown field `externalScopeId`"
    );
    assert!(envelope["diagnostics"][0]["hint"]
        .as_str()
        .unwrap()
        .contains("schema"));
    assert_eq!(
        serde_json::to_value(&result.content).unwrap(),
        original_content
    );
    assert_eq!(serde_json::to_value(&result).unwrap()["isError"], true);
}

#[test]
fn parameter_normalizer_leaves_other_responses_alone() {
    use rmcp::model::{CallToolResponse, CallToolResult, ContentBlock};

    let response = CallToolResponse::Complete(CallToolResult::error(vec![ContentBlock::text(
        "A domain operation failed",
    )]));
    let normalized = normalize_text_only_error(response, "harness_search");
    let CallToolResponse::Complete(result) = normalized else {
        panic!("result response remains complete");
    };
    assert!(result.structured_content.is_none());
}

#[test]
fn direct_json_response_keeps_text_and_adds_structured_envelope() {
    let envelope = json!({
        "state":"ok", "requestId":"r1", "scope":{"tool":"harness_symbols"},
        "diagnostics":[], "omitted":{}, "detail":null, "data":{"found":true}
    });
    let result = response_with_compact(envelope, false);
    let structured = result.structured_content.as_ref().unwrap();
    assert_eq!(structured["state"], "ok");
    assert_eq!(structured["scope"]["tool"], "harness_symbols");
    assert_eq!(structured["data"]["found"], true);
    assert!(structured.get("diagnostics").is_none());
    assert!(structured.get("omitted").is_none());
    assert!(structured.get("detail").is_none());

    let wire = serde_json::to_value(result).unwrap();
    let payload: Value =
        serde_json::from_str(wire["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(payload, json!({"found":true}));
}

#[test]
fn direct_json_error_keeps_text_and_structured_diagnostics() {
    let envelope = json!({
        "state":"error", "requestId":"r2", "scope":{"tool":"harness_explore"},
        "diagnostics":[{"code":"GRAPH_UNAVAILABLE"}],
        "detail":{"ref":"recover"}, "data":{"found":false}
    });
    let result = response_with_compact(envelope, false);
    let structured = result.structured_content.as_ref().unwrap();
    assert_eq!(structured["state"], "error");
    assert_eq!(structured["scope"]["tool"], "harness_explore");
    assert_eq!(structured["diagnostics"][0]["code"], "GRAPH_UNAVAILABLE");
    assert_eq!(structured["detail"]["ref"], "recover");

    let wire = serde_json::to_value(result).unwrap();
    assert_eq!(wire["isError"], true);
    let payload: Value =
        serde_json::from_str(wire["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(payload["state"], "error");
    assert_eq!(payload["requestId"], "r2");
    assert_eq!(payload["diagnostics"][0]["code"], "GRAPH_UNAVAILABLE");
    assert_eq!(payload["detail"]["ref"], "recover");
    assert_eq!(payload["found"], false);
}

#[test]
fn oversized_batch_with_detail_ref_renders_inline_summaries_for_every_job() {
    let envelope = json!({
        "state": "error",
        "scope": {"tool": "harness_batch"},
        "omitted": {"truncated": true},
        "detail": {"ref": "batch-detail-ref", "complete": false},
        "diagnostics": [{"code": "BATCH_PARTIAL", "message": "One or more read jobs did not succeed; inspect results"}],
        "data": {
            "results": [
                {
                    "index": 0,
                    "tool": "harness_search",
                    "target": "needle in src",
                    "state": "ok",
                    "omitted": {"matches": 24},
                    "detail": {"ref": "search-detail-ref"},
                    "data": {
                        "totalMatches": 30,
                        "matchedFiles": 4,
                        "previewTruncated": true,
                        "matches": [
                            {"file": "src/a.rs", "line": 12, "text": "let needle = 1;"},
                            {"file": "src/b.rs", "line": 44, "text": "use needle;"}
                        ]
                    }
                },
                {
                    "index": 1,
                    "tool": "harness_explore",
                    "target": "src/a.rs::process",
                    "state": "ok",
                    "detail": {"ref": "explore-detail-ref"},
                    "data": {
                        "view": "summary",
                        "indexFreshness": "fresh",
                        "fileFreshness": {"status": "fresh"},
                        "definitions": [
                            {
                                "path": "src/a.rs",
                                "role": "target",
                                "symbol": {"name": "process", "start": 10, "end": 25, "signature": "pub fn process()"}
                            }
                        ],
                        "graph": {
                            "nodeCount": 3,
                            "edgeCount": 2,
                            "relationCount": 2,
                            "relations": [
                                {"source": "caller_fn", "target": "process", "kind": "calls"}
                            ]
                        }
                    }
                },
                {
                    "index": 2,
                    "tool": "harness_read",
                    "target": "src/missing.rs:1-50",
                    "state": "error",
                    "diagnostics": [{"code": "PATH", "message": "file does not exist"}],
                    "data": {}
                }
            ]
        }
    });

    let result = response_with_presentation(envelope, false);
    let wire = serde_json::to_value(result).unwrap();
    assert_eq!(wire["isError"], true);
    let text = wire["content"][0]["text"].as_str().unwrap();

    for expected in [
        "## Batch · 3 results · 1 failed",
        "### 1 · harness_search · ok · `needle in src`",
        "## Search matches · 2 shown (30 total across 4 file(s))",
        "12 | let needle = 1;",
        "44 | use needle;",
        "detail.ref: search-detail-ref",
        "### 2 · harness_explore · ok · `src/a.rs::process`",
        "## Explore (summary) · 1 target(s), 0 related · 2 relation(s)",
        "`caller_fn` —[calls]→ `process`",
        "detail.ref: explore-detail-ref",
        "### 3 · harness_read · error · `src/missing.rs:1-50`",
        "**Status:** error (1 diagnostic(s))",
        "> PATH: file does not exist",
        "> BATCH_PARTIAL: One or more read jobs did not succeed; inspect results",
        "> detail.ref: batch-detail-ref",
    ] {
        assert!(text.contains(expected), "missing `{expected}` in:\n{text}");
    }
}

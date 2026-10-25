use super::{clean_empty_metadata, response_with_options};
use serde_json::json;

fn symbols_payload(envelope: serde_json::Value) -> (serde_json::Value, serde_json::Value) {
    let result = response_with_options(envelope, false, false);
    assert!(result.structured_content.is_none());
    assert_eq!(result.content.len(), 1);
    let wire = serde_json::to_value(result).unwrap();
    let payload = serde_json::from_str(wire["content"][0]["text"].as_str().unwrap()).unwrap();
    (payload, wire)
}

#[test]
fn symbols_sends_only_one_direct_json_without_flags() {
    let original = json!({
        "state":"ok","requestId":"correlation","scope":{"tool":"harness_symbols","path":"a.rs"},
        "diagnostics":[],"omitted":{},"detail":null,"data":{
            "path":"a.rs","hash":"sha","syntaxOnly":true,
            "symbol":{"name":"foo","kind":"function_item","container":"",
                "start":1,"end":3,"signature":"fn foo() …","symbolId":"id"},
            "anchors":{"before":null,"after":null},"text":"fn foo() {}"
        }
    });
    let (payload, wire) = symbols_payload(original.clone());
    assert_eq!(payload, json!({
        "path":"a.rs","syntaxOnly":true,
        "symbol":{"name":"foo","kind":"function_item","range":[1,3],"symbolId":"id"},
        "text":"fn foo() {}"
    }));
    assert_ne!(wire["isError"], true);
    assert!(payload.to_string().len() < original.to_string().len() / 2);
}

#[test]
fn symbols_direct_json_preserves_failures_and_recovery() {
    for state in ["ok", "error", "timeout", "cancelled"] {
        let diagnostics = json!([{"code":"AMBIGUOUS_SYMBOL","message":"select a candidate"}]);
        let detail = json!({"ref":"saved","complete":false,"nextCall":{"tool":"harness_detail"}});
        let (payload, wire) = symbols_payload(json!({
            "state":state,"requestId":"trace","scope":{"tool":"harness_symbols","path":"a.rs"},
            "diagnostics":diagnostics,"omitted":{"matches":3},"detail":detail,
            "data":{"candidates":[{"name":"foo","container":"A","symbolId":"id"}]}
        }));
        assert_eq!(payload["path"], "a.rs");
        assert_eq!(payload["diagnostics"], diagnostics);
        assert_eq!(payload["detail"], detail);
        assert_eq!(payload["omitted"], json!({"matches":3}));
        assert_eq!(payload["candidates"][0]["symbolId"], "id");
        if state == "ok" {
            assert!(payload.get("state").is_none());
            assert!(payload.get("requestId").is_none());
            assert_ne!(wire["isError"], true);
        } else {
            assert_eq!(payload["state"], state);
            assert_eq!(payload["requestId"], "trace");
            assert_eq!(wire["isError"], true);
        }
    }
}

#[test]
fn removes_null_results_from_read_search_and_batch_envelopes() {
    for tool in ["harness_read", "harness_search", "harness_batch"] {
        let mut response = json!({
            "scope": {"tool": tool},
            "data": {
                "results": null,
                "_detail": {"results": null},
                "optional": null
            }
        });

        clean_empty_metadata(&mut response);

        assert_eq!(
            response,
            json!({
                "scope": {"tool": tool},
                "data": {"optional": null}
            })
        );
    }

    let mut payload = json!({
        "scope": {"tool": "harness_sql"},
        "data": {"rows": [{"results": null}], "_detail": {"results": null}}
    });
    let expected = payload.clone();
    clean_empty_metadata(&mut payload);
    assert_eq!(payload, expected);

    let mut populated = json!({
        "scope": {"tool": "harness_search"},
        "data": {
            "results": [{"match": 1}],
            "_detail": {"results": [{"match": 2}], "cursor": "next"}
        }
    });
    let expected = populated.clone();
    clean_empty_metadata(&mut populated);
    assert_eq!(populated, expected);
}

#[test]
fn browser_screenshot_becomes_image_content_without_duplicate_base64() {
    let result = response_with_options(
        json!({
            "state": "ok",
            "scope": {"tool": "harness_browser"},
            "data": {
                "action": "screenshot",
                "mimeType": "image/png",
                "dataBase64": "aGVsbG8=",
                "width": 12,
                "height": 8
            }
        }),
        false,
        false,
    );

    assert!(matches!(
        result.content.as_slice(),
        [rmcp::model::ContentBlock::Image(image)]
            if image.data == "aGVsbG8=" && image.mime_type == "image/png"
    ));
    let wire = serde_json::to_value(result).unwrap();
    assert!(
        wire["structuredContent"]["data"]
            .get("dataBase64")
            .is_none()
    );
    assert_eq!(wire["structuredContent"]["data"]["width"], 12);
    assert_eq!(wire["structuredContent"]["data"]["height"], 8);
}

#[test]
fn explore_single_json_preserves_evidence_and_compacts_summary() {
    let original = json!({
        "state":"ok","scope":{"tool":"harness_explore"},"requestId":"trace",
        "omitted":{"truncated":true},"detail":{"ref":"saved"},
        "diagnostics":[{"code":"INDEX_STALE"}],"data":{
            "view":"summary","definitionView":"signature","text":"duplicate signature preview",
            "reuseGuidance":"static instructions","editReady":false,"absenceProven":false,
            "definitions":[{"path":"a.rs","hash":"sha","role":"target","sourceStatus":"signature_only",
                "symbol":{"name":"f","start":2,"end":4,"container":"","signature":"/// docs\nfn f()","symbolId":"id"},
                "nextCall":{"tool":"harness_symbols"}}],
            "selection":{"filePath":"a.rs","container":null,"multipleTargets":false},
            "contextSelection":{"policy":"static policy","requestedSymbols":["f"],"maxFiles":4,"maxRelatedSymbols":4,"relatedReturned":0,"relatedOmitted":2},
            "unresolved":[],"candidates":[],"candidatesOmitted":3,
            "graph":{"relations":[{"source":"f","target":"g","kind":"calls"}],"relationsOmitted":1,"truncation":{"depthBoundary":true}},
            "fileFreshness":{"status":"stale","staleFiles":1},"indexFreshness":"not_certified"
        }
    });
    let (payload, wire) = symbols_payload(original.clone());
    assert_ne!(wire["isError"], true);
    let with_legacy_flags = serde_json::to_value(response_with_options(original.clone(), true, true)).unwrap();
    assert_eq!(wire, with_legacy_flags, "graph callers need no presentation flags");
    assert!(payload.get("text").is_none());
    assert!(payload.get("reuseGuidance").is_none());
    assert_eq!(payload["definitions"][0]["symbol"]["signature"], "/// docs\nfn f()");
    assert_eq!(payload["definitions"][0]["symbol"]["range"], json!([2,4]));
    assert_eq!(payload["definitions"][0]["hash"], "sha");
    assert_eq!(payload["definitions"][0]["nextCall"]["tool"], "harness_symbols");
    for key in ["graph", "fileFreshness", "indexFreshness", "candidatesOmitted", "absenceProven", "editReady"] {
        assert_eq!(payload[key], original["data"][key]);
    }
    for key in ["diagnostics", "detail", "omitted"] {
        assert_eq!(payload[key], original[key]);
    }
    assert_eq!(payload["contextSelection"]["relatedOmitted"], 2);
    assert!(payload.to_string().len() < original.to_string().len());
}

#[test]
fn explore_full_keeps_source_and_failures_keep_diagnostics() {
    let body = "/// documentation\nfn f() {}";
    let (payload, _) = symbols_payload(json!({
        "state":"ok","scope":{"tool":"harness_explore"},"data":{
            "view":"full","definitionView":"full","text":body,"graph":{},
            "contextTruncation":{"definitionBodies":true},"unresolved":[{"symbol":"g"}]
        }
    }));
    assert_eq!(payload["text"], body);
    assert_eq!(payload["contextTruncation"]["definitionBodies"], true);
    assert_eq!(payload["unresolved"][0]["symbol"], "g");
    for state in ["error", "timeout", "cancelled"] {
        let (payload, wire) = symbols_payload(json!({
            "state":state,"requestId":"trace","scope":{"tool":"harness_explore"},
            "diagnostics":[{"code":"INDEX"}],"data":{"indexDiagnostic":{"stage":"schema"}}
        }));
        assert_eq!(wire["isError"], true);
        assert_eq!(payload["state"], state);
        assert_eq!(payload["requestId"], "trace");
        assert_eq!(payload["indexDiagnostic"]["stage"], "schema");
        assert_eq!(payload["diagnostics"][0]["code"], "INDEX");
    }
}

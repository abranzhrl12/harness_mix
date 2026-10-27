use super::{clean_empty_metadata, response_with_options};
use serde_json::json;

fn symbols_payload(envelope: serde_json::Value) -> (serde_json::Value, serde_json::Value) {
    let result = response_with_options(envelope, false, false);
    assert!(result.structured_content.is_some());
    assert_eq!(result.content.len(), 1);
    let wire = serde_json::to_value(result).unwrap();
    let payload = serde_json::from_str(wire["content"][0]["text"].as_str().unwrap()).unwrap();
    (payload, wire)
}

fn wire_of(envelope: serde_json::Value) -> serde_json::Value {
    serde_json::to_value(response_with_options(envelope, false, false)).unwrap()
}

#[test]
fn symbols_read_renders_a_lean_source_channel_and_keeps_the_canonical_envelope() {
    let wire = wire_of(json!({
        "state":"ok","requestId":"correlation","scope":{"tool":"harness_symbols","path":"a.rs"},
        "diagnostics":[],"omitted":{},"detail":null,"data":{
            "path":"a.rs","syntaxOnly":true,
            "symbol":{"name":"foo","kind":"function_item","container":"",
                "start":1,"end":3,"signature":"fn foo() …","symbolId":"id"},
            "anchors":{"before":null,"after":null},"text":"fn foo() {}"
        }
    }));
    assert_ne!(wire["isError"], true);
    // Canonical envelope: the reshaped model contract (range pairing, no
    // display hash/anchors/signature duplication for full reads).
    let data = &wire["structuredContent"]["data"];
    assert_eq!(data["symbol"]["range"], json!([1, 3]));
    assert_eq!(data["text"], "fn foo() {}");
    // Reading channel: language-tagged numbered source.
    let text = wire["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("## Symbol foo · a.rs"), "{text}");
    assert!(text.contains("```rust\n1 | fn foo() {}"), "{text}");
}

#[test]
fn read_renders_a_lean_numbered_channel_with_the_line_range() {
    let wire = wire_of(json!({
        "state":"ok","requestId":"r","scope":{"tool":"harness_read","path":"src/app.cs"},
        "diagnostics":[],"data":{"path":"src/app.cs","range":[15,16],"text":"using A;\r\nusing B;"}
    }));
    assert_ne!(wire["isError"], true);
    let text = wire["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("## Read src/app.cs · line 15"), "{text}");
    assert!(text.contains("```csharp"), "{text}");
    assert!(text.contains("15 | using A;\n16 | using B;"), "{text}");
    assert_eq!(
        wire["structuredContent"]["data"]["text"],
        json!("using A;\r\nusing B;")
    );
}

#[test]
fn outline_renders_a_markdown_table_from_items() {
    let wire = wire_of(json!({
        "state":"ok","scope":{"tool":"harness_symbols","path":"p.cs"},
        "data":{"path":"p.cs","items":[
            {"line":15,"kind":"method","name":"Anticipo_Cobrar","signature":"public long Anticipo_Cobrar(int a)","symbolId":"i1"},
            {"line":43,"kind":"method","name":"NumeroOrden_Listar","signature":"public List<E_NumeroOrden> NumeroOrden_Listar()","symbolId":"i2"}
        ],"omitted":0,"filtered":0}
    }));
    let text = wire["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("## Outline p.cs · 2 items"), "{text}");
    assert!(text.contains("| Line | Kind | Name | Signature |"), "{text}");
    assert!(text.contains("| 15 | method | Anticipo_Cobrar | public long Anticipo_Cobrar(int a) |"), "{text}");
    assert!(text.contains("| 43 | method | NumeroOrden_Listar |"), "{text}");
}

#[test]
fn sql_query_rows_render_a_markdown_table_and_keep_canonical_rows() {
    let rows = serde_json::json!([
        {"codigo_venta": "11841518", "serie_doc": "B011", "numero_doc": "00009563", "total": "42.37", "estado": "C"}
    ]);
    let wire = wire_of(json!({
        "state":"ok","scope":{"tool":"harness_sql","path":"web"},
        "diagnostics":[],"data":{"engine":"sqlserver","rowCount":1,"rows":rows,"truncated":false,
            "readOnlyPreflightPassed":true}
    }));
    let text = wire["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("## Query result · 1 row(s)"), "{text}");
    // Column order follows the row map (sorted), so assert per-column.
    for header in ["codigo_venta", "serie_doc", "numero_doc", "total", "estado"] {
        assert!(text.contains(&format!("| {header} ")), "{text}");
    }
    for value in ["11841518", "B011", "00009563", "42.37", "C"] {
        assert!(text.contains(value), "{text}");
    }
    assert_eq!(
        wire["structuredContent"]["data"]["rows"][0]["codigo_venta"],
        json!("11841518")
    );
}

#[test]
fn sql_describe_renders_a_column_table() {
    let wire = wire_of(json!({
        "state":"ok","scope":{"tool":"harness_sql"},
        "data":{"engine":"sqlserver","kind":"relation","name":"Anticipos","resolvedName":"[dbo].[Anticipos]",
            "objectId":17,"columns":[{"column":"id","type":"int","maxLength":null,"nullable":false},
                {"column":"nombre","type":"varchar","maxLength":-1,"nullable":true}],
            "truncated":false}
    }));
    let text = wire["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("## Describe [dbo].[Anticipos] (2 column(s))"), "{text}");
    assert!(text.contains("| id | int | - | NO |"), "{text}");
    assert!(text.contains("| nombre | varchar | -1 | YES |"), "{text}");
    assert_eq!(wire["structuredContent"]["data"]["columns"].as_array().unwrap().len(), 2);
}

#[test]
fn sql_routine_renders_parameter_table_and_definition_in_a_sql_fence() {
    let wire = wire_of(json!({
        "state":"ok","scope":{"tool":"harness_sql"},
        "data":{"engine":"sqlserver","kind":"routine","name":"Anticipo_Cobrar","resolvedName":"[dbo].[Anticipo_Cobrar]",
            "objectId":3,"definition":"CREATE PROCEDURE dbo.Anticipo_Cobrar AS",
            "parameters":[{"name":"@comprobante","typeName":"int","isOutput":false}],
            "parameterMetadataSource":"sys.parameters","parametersComplete":true,"truncated":false}
    }));
    let text = wire["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("## Define [dbo].[Anticipo_Cobrar] · parameters (1)"), "{text}");
    assert!(text.contains("| @comprobante | int | IN |"), "{text}");
    assert!(text.contains("```sql\nCREATE PROCEDURE dbo.Anticipo_Cobrar AS\n```"), "{text}");
    assert_eq!(
        wire["structuredContent"]["data"]["definition"],
        json!("CREATE PROCEDURE dbo.Anticipo_Cobrar AS")
    );
}

#[test]
fn git_diff_wraps_the_patch_in_a_diff_fence() {
    let wire = wire_of(json!({
        "state":"ok","scope":{"tool":"harness_git","path":"."},
        "data":{"path":"tracked.txt","comparison":"HEAD:worktree","changed":true,"binary":false,
            "hunks":["@@ -1,1 +1,1 @@"],"totalLines":6,
            "preview":"@@ -1,1 +1,1 @@\n-old line\n+new line","truncated":false,
            "files":{"changed":1,"included":1},"note":"Tracked changes only"}
    }));
    let text = wire["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("## Git diff tracked.txt · HEAD:worktree · 6 line(s)"), "{text}");
    assert!(text.contains("```diff\n@@ -1,1 +1,1 @@\n-old line\n+new line\n```"), "{text}");
    assert!(
        wire["structuredContent"]["data"]["preview"]
            .as_str()
            .unwrap()
            .contains("-old line")
    );
}

#[test]
fn git_status_renders_grouped_entries() {
    let wire = wire_of(json!({
        "state":"ok","scope":{"tool":"harness_git"},
        "data":{"branch":"main","head":"abc123def456","upstream":"origin/main","ahead":1,"behind":0,
            "entries":[{"path":"modified.txt","xy":"M ","kind":"tracked"},
                {"path":"new.txt","xy":"??","kind":"untracked"}],
            "counts":{"staged":1,"modified":0,"untracked":1}}
    }));
    let text = wire["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("## Git status · branch main · head abc123def456 · upstream origin/main · 2 file(s)"), "{text}");
    assert!(text.contains("### Tracked (staged or modified)"), "{text}");
    assert!(text.contains("- modified.txt (M )"), "{text}");
    assert!(text.contains("### Untracked"), "{text}");
    assert!(text.contains("- new.txt (??)"), "{text}");
    assert_eq!(wire["structuredContent"]["data"]["counts"]["untracked"], 1);
}

#[test]
fn search_text_matches_render_grouped_numbered_lines() {
    let wire = wire_of(json!({
        "state":"ok","scope":{"tool":"harness_search"},
        "data":{"matches":[
            {"file":"src/a.cs","line":12,"text":"needle in line"},
            {"file":"src/b.rs","line":5,"text":"rust needle"},
            {"file":"src/a.cs","line":20,"text":"needle again"}
        ],"complete":false,"offset":0,"warnings":["limited"]},
        "detail":{"ref":"saved","complete":false}
    }));
    let text = wire["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("## Search matches · 3 shown"), "{text}");
    assert!(text.contains("### src/a.cs\n\n```csharp\n12 | needle in line\n20 | needle again\n```"), "{text}");
    assert!(text.contains("### src/b.rs\n\n```rust\n5 | rust needle\n```"), "{text}");
    assert_eq!(wire["structuredContent"]["data"]["warnings"][0], json!("limited"));
    assert_eq!(wire["structuredContent"]["detail"]["ref"], json!("saved"));
    assert_eq!(
        wire["structuredContent"]["data"]["matches"][1]["line"],
        json!(5)
    );
}

#[test]
fn search_file_paths_render_as_a_readable_list() {
    let wire = wire_of(json!({
        "state":"ok","scope":{"tool":"harness_search"},
        "data":{"paths":["src/a.cs","src/b.rs"],"complete":true}
    }));
    let text = wire["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("## Search results · 2 file(s)"), "{text}");
    assert!(text.contains("- src/a.cs\n- src/b.rs\n"), "{text}");
    assert_eq!(wire["structuredContent"]["data"]["paths"][1], json!("src/b.rs"));
}

#[test]
fn search_with_no_matches_renders_an_empty_summary() {
    let wire = wire_of(json!({
        "state":"ok","scope":{"tool":"harness_search"},
        "data":{"matches":[],"complete":true,"warnings":[]}
    }));
    let text = wire["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("## Search matches · 0 shown"), "{text}");
    assert!(text.contains("No matches found."), "{text}");
    assert_eq!(wire["structuredContent"]["data"]["matches"], json!([]));
}

#[test]
fn search_text_does_not_silently_clip_long_matches_or_close_embedded_fences() {
    let long = "x".repeat(450);
    let wire = wire_of(json!({
        "state":"ok","scope":{"tool":"harness_search"},
        "data":{"matches":[
            {"file":"src/a.rs","line":12,"text":format!("{long} ``` embedded")}
        ]}
    }));
    let text = wire["content"][0]["text"].as_str().unwrap();
    assert!(text.contains(&long), "match text was clipped");
    assert!(text.contains("````rust\n12 | "), "fence did not protect embedded backticks: {text}");
}

#[test]
fn non_match_and_error_payloads_keep_the_legacy_channels() {
    // harness_search without text rows (name matches) stays direct JSON.
    let wire = wire_of(json!({
        "state":"ok","scope":{"tool":"harness_search"},
        "data":{"matches":[{"file":"f.rs","name":"foo","line":3,"endLine":4}]}
    }));
    let payload: serde_json::Value =
        serde_json::from_str(wire["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(payload["data"]["matches"][0]["name"], json!("foo"));
    // SQL status and mapping stay direct JSON.
    let wire = wire_of(json!({
        "state":"ok","scope":{"tool":"harness_sql"},
        "data":{"engine":"sqlserver","configured":true,"status":"ready","readOnly":true}
    }));
    let payload: serde_json::Value =
        serde_json::from_str(wire["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(payload["data"]["status"], json!("ready"));
    // Failures keep the structured error contract for every lean tool.
    let wire = wire_of(json!({
        "state":"error","scope":{"tool":"harness_read"},
        "diagnostics":[{"code":"PATH"}],"data":{}
    }));
    let payload: serde_json::Value =
        serde_json::from_str(wire["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(payload["state"], json!("error"));
    assert_eq!(wire["isError"], true);
}

#[test]
fn symbols_direct_json_and_envelope_preserve_failures_and_recovery() {
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
fn explore_text_and_envelope_preserve_evidence_and_compact_summary() {
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
fn explore_full_uses_the_lean_text_channel_and_failures_keep_diagnostics() {
    let body = "/// documentation\nfn f() {}";
    let result = serde_json::to_value(response_with_options(
        json!({
            "state":"ok","scope":{"tool":"harness_explore"},"requestId":"trace","detail":{"ref":"saved"},
            "data":{"view":"full","definitionView":"full","text":body,"graph":{},
                "contextTruncation":{"definitionBodies":true},"unresolved":[{"symbol":"g"}]}
        }),
        false,
        false,
    ))
    .unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("## Graph context"));
    // The source appears once in the reading channel; the full source stays
    // canonical in structuredContent for envelope-first clients.
    assert_eq!(text.matches("fn f() {}").count(), 1);
    assert_eq!(
        result["structuredContent"]["data"]["text"],
        json!(body)
    );
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

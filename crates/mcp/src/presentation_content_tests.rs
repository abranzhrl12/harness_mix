use super::{clean_empty_metadata, response_with_options};
use serde_json::json;

#[test]
fn structured_explore_keeps_single_lineage_and_evidence() {
    let trace = json!({"identifier":"field","layers":[{"source":"unique body"}]});
    let wire = serde_json::to_value(super::structured_presentation(json!({
        "state":"ok","scope":{"tool":"harness_explore"},"detail":{"ref":"saved"},
        "omitted":{"lines":2},"data":{"text":"unique body","dataTrace":trace,"crossLayerTrace":trace,
        "fileFreshness":{"status":"changed"},"truncated":true}
    }))).unwrap();
    let text = wire["content"][0]["text"].as_str().unwrap();
    assert!(!text.contains("unique body"));
    assert!(text.contains("Partial") && text.contains("saved"));
    assert_eq!(wire["structuredContent"]["data"]["dataTrace"]["identifier"], "field");
    assert!(wire["structuredContent"]["data"].get("crossLayerTrace").is_none());
    assert_eq!(wire["structuredContent"]["data"]["fileFreshness"]["status"], "changed");
}

#[test]
fn structured_presentation_keeps_source_and_notices_without_repeating_body() {
    let body = "unique source body";
    let envelope = json!({"state":"ok","scope":{"tool":"harness_batch"},
        "detail":{"ref":"saved"},"omitted":{"lines":4},
        "data":{"results":[{"tool":"harness_read","state":"ok","data":{"path":"a.rs","text":body}},
            {"tool":"harness_search","state":"error","diagnostics":[{"code":"PATH","message":"missing"}]}]}});
    let result = serde_json::to_value(super::structured_presentation(envelope)).unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(!text.contains(body));
    for item in ["PATH", "missing", "saved", "Partial"] {
        assert!(text.contains(item), "{text}");
    }
    assert_eq!(
        result["structuredContent"]["data"]["results"][0]["data"]["text"],
        body
    );
}

#[test]
fn structured_presentation_preserves_search_context_and_bootstrap_session() {
    let result = serde_json::to_value(super::structured_presentation(json!({
        "state":"ok","scope":{"tool":"harness_search"},
        "data":{"matches":[{"hash":"hash","context":{"text":"source text"}}]}
    })))
    .unwrap();
    assert_eq!(
        result["structuredContent"]["data"]["matches"][0]["context"]["text"],
        "source text"
    );
    assert!(
        !result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("source text")
    );
    let boot = serde_json::to_value(super::structured_presentation(json!({
        "state":"ok","scope":{"tool":"harness_bootstrap"},
        "data":{"root":"E:/project","session":"session-id"}
    })))
    .unwrap();
    let text = boot["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("E:/project") && text.contains("session-id"));
}

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
fn detail_renders_saved_batch_and_preserves_raw_payload() {
    let raw = json!({"results":[{"tool":"harness_read","result":{"state":"ok",
        "scope":{"tool":"harness_read"},"data":{"path":"a.rs","start":3,"end":4,"text":"first\nsecond"}}}]}).to_string();
    let wire = wire_of(json!({"state":"ok","scope":{"tool":"harness_detail"},
        "data":{"ref":"saved","offset":0,"text":raw,"nextOffset":null,"totalCharacters":100}}));
    let text = wire["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("a.rs") && text.contains("first") && text.contains("second"), "{text}");
    assert!(!text.contains("\\nsecond"), "{text}");
    assert_eq!(wire["structuredContent"]["data"]["text"], raw);
}

#[test]
fn detail_partial_page_keeps_fragment_and_continuation() {
    let raw = "{\"incomplete\":";
    let wire = wire_of(json!({"state":"ok","scope":{"tool":"harness_detail"},
        "data":{"ref":"saved","offset":0,"text":raw,"nextOffset":12,"totalCharacters":50}}));
    let text = wire["content"][0]["text"].as_str().unwrap();
    assert!(text.contains(raw) && text.contains("12") && text.contains("saved"), "{text}");
    assert_eq!(wire["structuredContent"]["data"]["text"], raw);
}

#[test]
fn detail_first_partial_read_decodes_source_preview() {
    let raw = r#"{"results":[{"tool":"harness_read","result":{"data":{"path":"a.rs","text":"first\nsecond"#;
    let wire = wire_of(json!({"state":"ok","scope":{"tool":"harness_detail"},
        "data":{"ref":"saved","offset":0,"text":raw,"nextOffset":80,"totalCharacters":200}}));
    let text = wire["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("a.rs") && text.contains("first\nsecond"), "{text}");
    assert!(text.contains("partial preview") && text.contains("80"), "{text}");
    assert_eq!(wire["structuredContent"]["data"]["text"], raw);
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
fn symbols_signature_without_source_renders_signature_location_and_selector() {
    let wire = wire_of(json!({
        "state":"ok","scope":{"tool":"harness_symbols","path":"src/lib.rs"},
        "data":{"path":"src/lib.rs","hash":"full-hash","text":null,"notice":"Use full read for source.",
            "symbol":{"name":"answer","kind":"function_item","start":12,"end":14,
                "signature":"pub fn answer() -> u32","symbolId":"rust:function:answer"}}
    }));
    let text = wire["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("answer at src/lib.rs:12"), "{text}");
    assert!(text.contains("pub fn answer() -> u32"), "{text}");
    assert!(text.contains("rust:function:answer"), "{text}");
    assert!(text.contains("Use full read for source."), "{text}");
    assert_eq!(wire["structuredContent"]["data"]["hash"], "full-hash");
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
    assert!(
        text.contains("| Line | Kind | Name | Signature |"),
        "{text}"
    );
    assert!(
        text.contains("| 15 | method | Anticipo_Cobrar | public long Anticipo_Cobrar(int a) |"),
        "{text}"
    );
    assert!(
        text.contains("| 43 | method | NumeroOrden_Listar |"),
        "{text}"
    );
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
fn sql_empty_query_and_shortened_cells_are_explicit() {
    let empty = wire_of(json!({
        "state":"ok","scope":{"tool":"harness_sql"},
        "data":{"rowCount":0,"rows":[],"columns":["id","name"]}
    }));
    let text = empty["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("0 row(s)"), "{text}");
    assert!(text.contains("No rows returned"), "{text}");
    assert!(text.contains("id, name"), "{text}");

    let value = "x".repeat(120);
    let clipped = wire_of(json!({
        "state":"ok","scope":{"tool":"harness_sql"},
        "data":{"rowCount":1,"rows":[{"value":value}],"truncated":false}
    }));
    let text = clipped["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Some cell values are shortened"), "{text}");
    assert_eq!(
        clipped["structuredContent"]["data"]["rows"][0]["value"]
            .as_str()
            .unwrap()
            .len(),
        120
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
    assert!(
        text.starts_with("## Describe [dbo].[Anticipos] (2 column(s))"),
        "{text}"
    );
    assert!(text.contains("| id | int | - | NO |"), "{text}");
    assert!(text.contains("| nombre | varchar | -1 | YES |"), "{text}");
    assert_eq!(
        wire["structuredContent"]["data"]["columns"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
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
    assert!(
        text.starts_with("## Define [dbo].[Anticipo_Cobrar] · parameters (1)"),
        "{text}"
    );
    assert!(text.contains("| @comprobante | int | IN |"), "{text}");
    assert!(
        text.contains("```sql\nCREATE PROCEDURE dbo.Anticipo_Cobrar AS\n```"),
        "{text}"
    );
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
    assert!(
        text.starts_with("## Git diff tracked.txt · HEAD:worktree · 6 line(s)"),
        "{text}"
    );
    assert!(
        text.contains("```diff\n@@ -1,1 +1,1 @@\n-old line\n+new line\n```"),
        "{text}"
    );
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
    assert!(
        text.starts_with(
            "## Git status · branch main · head abc123def456 · upstream origin/main · 2 file(s)"
        ),
        "{text}"
    );
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
    assert!(
        text.contains("### src/a.cs\n\n```csharp\n12 | needle in line\n…\n20 | needle again\n```"),
        "{text}"
    );
    assert!(
        text.contains("### src/b.rs\n\n```rust\n5 | rust needle\n```"),
        "{text}"
    );
    assert_eq!(
        wire["structuredContent"]["data"]["warnings"][0],
        json!("limited")
    );
    assert_eq!(wire["structuredContent"]["detail"]["ref"], json!("saved"));
    assert_eq!(
        wire["structuredContent"]["data"]["matches"][1]["line"],
        json!(5)
    );
}

#[test]
fn search_renders_and_merges_overlapping_context_ranges() {
    let wire = wire_of(json!({
        "state":"ok","scope":{"tool":"harness_search"},
        "data":{"matches":[
            {"file":"src/a.rs","line":3,"text":"needle","hash":"full-digest",
                "context":{"start":2,"end":4,"text":"before\r\nneedle\r\nmiddle\r\n"}},
            {"file":"src/a.rs","line":4,"text":"second","context":{"start":4,"end":6,"text":"middle\r\nafter\r\nlast\r\n"}}
        ]}
    }));
    let text = wire["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("2 | before\n3 | needle\n4 | middle\n5 | after\n6 | last"),
        "{text}"
    );
    assert_eq!(text.matches("4 | middle").count(), 1, "{text}");
    assert_eq!(
        wire["structuredContent"]["data"]["matches"][0]["hash"],
        "full-digest"
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
    assert_eq!(
        wire["structuredContent"]["data"]["paths"][1],
        json!("src/b.rs")
    );
}

#[test]
fn search_file_size_and_syntax_results_render_their_distinct_row_shapes() {
    let sizes = wire_of(json!({
        "state":"ok","scope":{"tool":"harness_search"},
        "data":{"kind":"file_size","minLines":20,"files":[
            {"path":"src/a.rs","lines":21,"bytes":512}
        ]}
    }));
    let text = sizes["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("File size search"), "{text}");
    assert!(text.contains("| src/a.rs | 21 | 512 |"), "{text}");

    let syntax = wire_of(json!({
        "state":"ok","scope":{"tool":"harness_search"},
        "data":{"kind":"syntax","selector":"function_item:load","matches":[
            {"file":"src/a.rs","start":12,"end":15,"kind":"function_item",
                "name":"load","signature":"fn load()","symbolId":"rust:fn:load"}
        ]}
    }));
    let text = syntax["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Syntax search"), "{text}");
    assert!(
        text.contains("| src/a.rs | 12 | function_item | load | fn load() | rust:fn:load |"),
        "{text}"
    );
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
    assert!(
        text.contains("````rust\n12 | "),
        "fence did not protect embedded backticks: {text}"
    );
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
    let with_legacy_flags =
        serde_json::to_value(response_with_options(original.clone(), true, true)).unwrap();
    assert_eq!(
        wire, with_legacy_flags,
        "graph callers need no presentation flags"
    );
    assert!(payload.get("text").is_none());
    assert!(payload.get("reuseGuidance").is_none());
    assert_eq!(
        payload["definitions"][0]["symbol"]["signature"],
        "/// docs\nfn f()"
    );
    assert_eq!(payload["definitions"][0]["symbol"]["range"], json!([2, 4]));
    assert_eq!(payload["definitions"][0]["hash"], "sha");
    assert_eq!(
        payload["definitions"][0]["nextCall"]["tool"],
        "harness_symbols"
    );
    for key in [
        "graph",
        "fileFreshness",
        "indexFreshness",
        "candidatesOmitted",
        "absenceProven",
        "editReady",
    ] {
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
    assert_eq!(result["structuredContent"]["data"]["text"], json!(body));
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

#[test]
fn batch_with_explore_impact_sql_status_and_verify_renders_inline_summaries() {
    let wire = wire_of(json!({
        "state": "ok",
        "scope": {"tool": "harness_batch"},
        "detail": {"ref": "batch-ref"},
        "data": {
            "results": [
                {
                    "index": 0,
                    "tool": "harness_explore",
                    "target": "src/auth.rs::validate",
                    "state": "ok",
                    "data": {
                        "view": "impact",
                        "file": "src/auth.rs",
                        "symbol": "validate",
                        "indexFreshness": "fresh",
                        "fileFreshness": {"status": "fresh"},
                        "counts": {"callers": 2, "directCallers": 1, "files": 2},
                        "callers": [
                            {"symbol": "login", "file": "src/api.rs", "declarationLine": 18, "direct": true},
                            {"symbol": "handle_req", "file": "src/http.rs", "declarationLine": 55, "direct": false}
                        ],
                        "callersOmitted": 0
                    }
                },
                {
                    "index": 1,
                    "tool": "harness_sql",
                    "target": "status",
                    "state": "ok",
                    "data": {
                        "engine": "sqlserver",
                        "status": "ready",
                        "configured": true,
                        "readOnly": true
                    }
                },
                {
                    "index": 2,
                    "tool": "harness_verify",
                    "target": "rust (crates/mcp)",
                    "state": "ok",
                    "data": {
                        "action": "test",
                        "status": "failed",
                        "errorCount": 1,
                        "warningCount": 0,
                        "errors": [
                            {"code": "E0308", "file": "src/lib.rs", "line": 42, "message": "mismatched types"}
                        ]
                    }
                }
            ]
        }
    }));

    let text = wire["content"][0]["text"].as_str().unwrap();
    for expected in [
        "## Batch · 3 results · 0 failed",
        "### 1 · harness_explore · ok · `src/auth.rs::validate`",
        "## Impact · validate at src/auth.rs · 2 caller(s) (1 direct) across 2 file(s)",
        "`login` (src/api.rs:18) · direct",
        "### 2 · harness_sql · ok · `status`",
        "## SQL status (sqlserver) · ready · configured=true · readOnly=true",
        "### 3 · harness_verify · ok · `rust (crates/mcp)`",
        "## Verify (test) · failed · 1 error(s), 0 warning(s)",
        "src/lib.rs:42 E0308: mismatched types",
        "> detail.ref: batch-ref",
    ] {
        assert!(text.contains(expected), "missing `{expected}` in:\n{text}");
        let mut matches = Vec::new();
        let chunk = "    SqlConnection cn = new SqlConnection(cadena);\n    SqlCommand cmd = new SqlCommand(\"PA_Caja_Listar\", cn);\n    cmd.CommandType = CommandType.StoredProcedure;\n";
        for idx in 0..40 {
            let start = idx * 4 + 1;
            matches.push(json!({
                "file": "CapaDatos/P_Caja.cs",
                "line": start + 1,
                "text": "    SqlCommand cmd = new SqlCommand(\"PA_Caja_Listar\", cn);",
                "hash": "sha256-full",
                "context": {
                    "start": start,
                    "end": start + 2,
                    "text": chunk
                }
            }));
        }
        let wire2 = wire_of(json!({
            "state": "ok",
            "scope": {"tool": "harness_search"},
            "detail": {"ref": "search-snapshot-1"},
            "data": {
                "onlyMatching": true,
                "extracted": ["PA_Caja_Listar"],
                "matches": matches
            }
        }));
        let text2 = wire2["content"][0]["text"].as_str().unwrap();
        assert!(
            text2.contains("### Extracted unique matches (1)"),
            "{text2}"
        );
        assert!(text2.contains("- `PA_Caja_Listar`"), "{text2}");
        assert!(
            text2.contains("SqlCommand(\"PA_Caja_Listar\", cn);"),
            "{text2}"
        );
        assert_eq!(
            wire2["structuredContent"]["data"]["contextInReadableChannel"],
            json!(true)
        );
        assert!(
            wire2["structuredContent"]["data"]["matches"][0]["context"]
                .get("text")
                .is_none()
        );
        assert_eq!(
            wire2["structuredContent"]["data"]["matches"][0]["context"]["range"],
            json!([1, 3])
        );
        assert_eq!(
            wire2["structuredContent"]["data"]["matches"][0]["hash"],
            json!("sha256-full")
        );
    }
}

#[test]
fn normalize_text_only_error_extracts_missing_and_unknown_field_guidance() {
    use rmcp::model::{CallToolResponse, CallToolResult, ContentBlock};
    let raw = CallToolResponse::Complete(CallToolResult::error(vec![ContentBlock::text(
        "Failed to deserialize parameters: Missing required field `name`. Invalid harness_sql describe.",
    )]));
    let normalized = super::normalize_text_only_error(raw, "harness_sql");
    let CallToolResponse::Complete(res) = normalized else {
        panic!("expected complete");
    };
    let sc = res.structured_content.unwrap();
    let diag = &sc["diagnostics"][0];
    assert_eq!(diag["code"], "ARGUMENT");
    assert_eq!(diag["details"]["missingField"], "name");
    assert!(
        diag["hint"]
            .as_str()
            .unwrap()
            .contains("Missing required field `name`. REQUIRED by action: describe ->")
    );
}

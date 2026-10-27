    use super::shape_for_model;
    use serde_json::json;

    fn read(data: serde_json::Value) -> serde_json::Value {
        json!({"state":"ok","scope":{"tool":"harness_read"},"data":data})
    }

    #[test]
    fn read_drops_hash_and_anchors_and_pairs_the_range() {
        let mut envelope = read(json!({
            "path":"a.rs","start":3,"end":9,"totalLines":40,"hash":"abc","text":"x",
            "anchors":{"before":null,"after":null}
        }));
        shape_for_model(&mut envelope);
        assert_eq!(
            envelope["data"],
            json!({"path":"a.rs","range":[3,9],"totalLines":40,"text":"x"})
        );
    }

    #[test]
    fn other_tools_and_errors_pass_through_unchanged() {
        let mut unknown = json!({"scope":{"tool":"harness_unknown"},"data":{"hash":"h","start":1,"end":2}});
        shape_for_model(&mut unknown);
        assert_eq!(unknown["data"], json!({"hash":"h","start":1,"end":2}));
        assert_eq!(unknown["scope"]["tool"], "harness_unknown");

        let mut error = json!({"state":"error","scope":{"tool":"harness_read"},"data":{}});
        shape_for_model(&mut error);
        assert_eq!(error["data"], json!({}));
        assert_eq!(error["scope"]["tool"], "harness_read");
    }

    #[test]
    fn search_matches_ranges_are_paired_and_telemetry_pruned() {
        let mut search = json!({
            "scope": {"tool": "harness_search"},
            "data": {
                "scanned": 100,
                "bytesRead": 50000,
                "skippedFiles": [],
                "complete": true,
                "limits": {"contextLines": 20},
                "matches": [
                    {"file": "a.rs", "start": 10, "end": 20, "name": "foo", "hash": "fullsha"},
                    {"file": "b.rs", "line": 5, "context": {"start": 3, "end": 7}}
                ]
            }
        });
        shape_for_model(&mut search);
        assert_eq!(
            search["data"]["matches"][0]["range"],
            json!([10, 20])
        );
        assert!(search["data"]["matches"][0].get("start").is_none());
        assert!(search["data"]["matches"][0].get("hash").is_none());
        assert!(search["data"].get("scanned").is_none());
        assert!(search["data"].get("bytesRead").is_none());
        assert!(search["data"].get("skippedFiles").is_none());
        assert!(search["data"].get("complete").is_none());
        assert!(search["data"].get("limits").is_none());
        assert_eq!(
            search["data"]["matches"][1]["context"]["range"],
            json!([3, 7])
        );
    }

    #[test]
    fn symbols_full_read_is_compact() {
        let mut envelope = json!({"scope":{"tool":"harness_symbols"},"data":{
            "path":"a.rs","hash":"sha","syntaxOnly":true,
            "symbol":{"name":"foo","kind":"function_item","container":"",
                "start":10,"end":12,"signature":"fn foo() …","symbolId":"id"},
            "anchors":{"before":null,"after":null},"text":"fn foo() {}"
        }});
        shape_for_model(&mut envelope);
        assert_eq!(envelope["data"], json!({
            "path":"a.rs","hash":"sha","syntaxOnly":true,"text":"fn foo() {}",
            "symbol":{"name":"foo","kind":"function_item","range":[10,12],"symbolId":"id"}
        }));
    }

    #[test]
    fn symbols_keep_extraction_hashes_signatures_and_omission_evidence() {
        let mut envelope = json!({"scope":{"tool":"harness_symbols"},
            "omitted":{"items":2},"detail":{"ref":"detail"},"data":{
                "path":"Widget.vue","hash":"required-for-extraction","language":"vue",
                "items":[{"name":"foo","container":"Widget","signature":"foo()",
                    "start":1,"end":3,"symbolId":"id"}],
                "inventory":{"filtered":0,"omitted":2},"filtered":0,
                "selection":{"method":"lexical-ranking","maxChars":1000,"usedChars":100,"unit":"UTF-16"},
                "coverage":"syntax only","structure":{"omitted":true}
        }});
        shape_for_model(&mut envelope);
        assert_eq!(envelope["data"]["hash"], "required-for-extraction");
        assert_eq!(envelope["data"]["items"][0], json!({
            "name":"foo","container":"Widget","signature":"foo()","range":[1,3],"symbolId":"id"
        }));
        assert_eq!(envelope["data"]["selection"], json!({"method":"lexical-ranking"}));
        assert!(envelope["data"].get("filtered").is_none());
        assert_eq!(envelope["data"]["inventory"]["omitted"], 2);
        assert_eq!(envelope["omitted"], json!({"items":2}));
        assert_eq!(envelope["detail"], json!({"ref":"detail"}));
        assert_eq!(envelope["data"]["structure"], json!({"omitted":true}));

        let mut signature = json!({"scope":{"tool":"harness_symbols"},"data":{
            "path":"a.rs","hash":"sha","text":null,"truncated":true,
            "symbol":{"name":"foo","signature":"foo()","start":1,"end":3},
            "notice":"request full","editReady":false
        }});
        shape_for_model(&mut signature);
        assert_eq!(signature["data"]["symbol"]["signature"], "foo()");
        assert_eq!(signature["data"]["truncated"], true);
        assert_eq!(signature["data"]["notice"], "request full");
        assert_eq!(signature["data"]["editReady"], false);
    }

    #[test]
    fn partial_ranges_are_left_alone() {
        let mut envelope = read(json!({"start":1,"text":"x"}));
        shape_for_model(&mut envelope);
        assert_eq!(envelope["data"], json!({"start":1,"text":"x"}));
    }

    #[test]
    fn batch_children_are_shaped_individually() {
        let mut batch = json!({"scope":{"tool":"harness_batch"},"data":{"results":[
            read(json!({"start":1,"end":1,"hash":"h","text":"a"})),
            {"scope":{"tool":"harness_search"},"data":{"hash":"keep"}}
        ]}});
        shape_for_model(&mut batch);
        assert_eq!(batch["data"]["results"][0]["data"], json!({"range":[1,1],"text":"a"}));
        assert_eq!(batch["data"]["results"][1]["data"], json!({"hash":"keep"}));
    }


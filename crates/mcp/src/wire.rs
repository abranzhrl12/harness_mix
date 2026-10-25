//! Model-facing shape of tool results.
//!
//! The domain keeps the complete contract (full hashes, `start`/`end`, context
//! anchors). This layer removes what a model does not need, so those fields cost
//! no tokens. Each tool lists the steps it wants; adding or dropping a field is a
//! one-line change here and never touches an adapter.
use serde_json::{Map, Value};

/// One reshaping step over a tool's `data` object.
type Step = fn(&mut Map<String, Value>);

/// Steps applied to the `data` of each tool, in order.
fn steps(tool: &str) -> &'static [Step] {
    match tool {
        // The edit tool binds to the file it reads, so a read hash is redundant;
        // `anchors` repeat neighbouring lines nobody consumes.
        "harness_read" => &[drop_hash, drop_anchors, pair_range],
        "harness_search" => &[shape_search_matches],
        "harness_symbols" => &[shape_symbols],
        "harness_explore" => &[shape_explore],
        _ => &[],
    }
}

/// Reshape an envelope (and the child envelopes of a batch) for the model.
pub(super) fn shape_for_model(envelope: &mut Value) {
    if let Some(tool) = envelope["scope"]["tool"].as_str().map(str::to_owned) {
        if let Some(data) = envelope.get_mut("data").and_then(Value::as_object_mut) {
            for step in steps(&tool) {
                step(data);
            }
        }
    }
    // A batch carries one complete envelope per child.
    let children = envelope
        .get_mut("data")
        .and_then(|data| data.get_mut("results"))
        .and_then(Value::as_array_mut);
    if let Some(children) = children {
        for child in children {
            shape_for_model(child);
        }
    }

}

fn drop_hash(data: &mut Map<String, Value>) {
    data.remove("hash");
}

fn shape_explore(data: &mut Map<String, Value>) {
    // Only successful graph payloads have this shape. Diagnostic-only responses
    // (missing/invalid indexes, unresolved targets) must retain their evidence.
    if !data.get("graph").is_some_and(Value::is_object) {
        return;
    }
    let summary = data.get("definitionView").and_then(Value::as_str) == Some("signature");
    if summary {
        // sources::load renders only the same signatures and static guidance in
        // summary text; structured definitions retain the complete signatures.
        data.remove("text");
    }
    data.remove("reuseGuidance");
    // Runtime policy and budget accounting are not evidence. Keep omission counts.
    if let Some(selection) = data.get_mut("contextSelection").and_then(Value::as_object_mut) {
        for key in ["policy", "requestedSymbols", "maxRelatedSymbols", "maxFiles"] {
            selection.remove(key);
        }
    }
    if let Some(selection) = data.get_mut("selection").and_then(Value::as_object_mut) {
        selection.retain(|_, value| !value.is_null());
    }
    if let Some(definitions) = data.get_mut("definitions").and_then(Value::as_array_mut) {
        for definition in definitions {
            if let Some(symbol) = definition.get_mut("symbol").and_then(Value::as_object_mut) {
                shape_symbol(symbol, false);
            }
        }
    }
    for key in ["unresolved", "candidates"] {
        if data.get(key).and_then(Value::as_array).is_some_and(Vec::is_empty) {
            data.remove(key);
        }
    }
    if let Some(graph) = data.get_mut("graph").and_then(Value::as_object_mut) {
        // Traversal bookkeeping is recoverable from the stored detail graph.
        // Preserve direction, depth, hard limits and every truncation indicator.
        if let Some(traversal) = graph.get_mut("traversal").and_then(Value::as_object_mut) {
            for key in ["expandedNodes", "semantics"] {
                traversal.remove(key);
            }
        }
    }
}

fn drop_anchors(data: &mut Map<String, Value>) {
    data.remove("anchors");
}

fn shape_symbols(data: &mut Map<String, Value>) {
    // Preview payloads have their own contract; only shape reads and outlines.
    if !data.contains_key("symbol") && !data.contains_key("items") {
        return;
    }
    // Vue extraction requires expectedHash from an outline (including template IDs).
    let vue = data.get("language").and_then(Value::as_str) == Some("vue")
        || data.get("path").and_then(Value::as_str).is_some_and(|p| {
            p.rsplit('.').next().is_some_and(|ext| ext.eq_ignore_ascii_case("vue"))
        });
    if !vue {
        drop_hash(data);
    }
    drop_anchors(data);
    let full_source = data.get("text").is_some_and(Value::is_string);
    if let Some(symbol) = data.get_mut("symbol").and_then(Value::as_object_mut) {
        shape_symbol(symbol, full_source);
    }
    if let Some(items) = data.get_mut("items").and_then(Value::as_array_mut) {
        for item in items {
            if let Some(symbol) = item.as_object_mut() {
                shape_symbol(symbol, false);
            }
        }
    }
    // Keep ranking policy, but omit budget accounting and duplicate counts.
    if let Some(selection) = data.get_mut("selection").and_then(Value::as_object_mut) {
        selection.remove("maxChars");
        selection.remove("usedChars");
        selection.remove("unit");
    }
    if data.get("selection") == Some(&serde_json::json!({"method":"source-order"})) {
        data.remove("selection");
    }
    if data.get("filtered").is_some()
        && data.get("filtered") == data.get("inventory").and_then(|v| v.get("filtered"))
    {
        data.remove("filtered");
    }
}

fn shape_symbol(symbol: &mut Map<String, Value>, full_source: bool) {
    pair_range(symbol);
    if symbol.get("container").and_then(Value::as_str) == Some("") {
        symbol.remove("container");
    }
    if full_source {
        symbol.remove("signature");
    }
}

/// `start` + `end` become one `range: [start, end]`; other shapes are untouched.
fn pair_range(data: &mut Map<String, Value>) {
    if data.get("start").is_some_and(Value::is_u64) && data.get("end").is_some_and(Value::is_u64) {
        let (start, end) = (data.remove("start"), data.remove("end"));
        data.insert("range".into(), Value::Array(vec![start.unwrap(), end.unwrap()]));
    }
}

fn shape_search_matches(data: &mut Map<String, Value>) {
    // Drop execution noise and hardware metrics not needed by LLMs
    data.remove("scanned");
    data.remove("bytesRead");
    data.remove("skipped");
    data.remove("skippedFiles");
    data.remove("unvisitedFiles");
    data.remove("limited");
    data.remove("limits");
    data.remove("contextLines");
    if data.get("complete") == Some(&Value::Bool(true)) {
        data.remove("complete");
    }

    if let Some(matches) = data.get_mut("matches").and_then(Value::as_array_mut) {
        for row in matches {
            if let Some(row_map) = row.as_object_mut() {
                row_map.remove("hash");
                pair_range(row_map);
                if let Some(context) = row_map.get_mut("context").and_then(Value::as_object_mut) {
                    pair_range(context);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
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
            "path":"a.rs","syntaxOnly":true,"text":"fn foo() {}",
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
}

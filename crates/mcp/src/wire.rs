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

fn drop_anchors(data: &mut Map<String, Value>) {
    data.remove("anchors");
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
        let expected = unknown.clone();
        shape_for_model(&mut unknown);
        assert_eq!(unknown, expected);

        let mut error = json!({"state":"error","scope":{"tool":"harness_read"},"data":{}});
        let expected = error.clone();
        shape_for_model(&mut error);
        assert_eq!(error, expected);
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

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
        // Keep full digests: callers use them as stale-source guards when
        // applying a replacement or correlating a search result with source.
        "harness_read" => &[drop_anchors, pair_range],
        "harness_search" => &[shape_search_matches],
        "harness_symbols" => &[shape_symbols],
        "harness_explore" => &[shape_explore],
        _ => &[],
    }
}

/// Reshape an envelope (and the child envelopes of a batch) for the model.
pub(super) fn shape_for_model(envelope: &mut Value) {
    let tool = envelope["scope"]["tool"].as_str().map(str::to_owned);
    shape_tool_data(tool.as_deref(), envelope.get_mut("data"));
    // A batch carries one complete envelope per child.
    let children = envelope
        .get_mut("data")
        .and_then(|data| data.get_mut("results"))
        .and_then(Value::as_array_mut);
    if let Some(children) = children {
        for child in children {
            // Application batch children flatten `tool` at their root and do
            // not contain a nested envelope/scope. Shape their payload directly.
            if let Some(tool) = child["tool"].as_str().map(str::to_owned) {
                shape_tool_data(Some(&tool), child.get_mut("data"));
            } else {
                shape_for_model(child);
            }
        }
    }
}

fn shape_tool_data(tool: Option<&str>, data: Option<&mut Value>) {
    let (Some(tool), Some(data)) = (tool, data) else {
        return;
    };
    if let Some(data) = data.as_object_mut() {
        for step in steps(tool) {
            step(data);
        }
    }
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
    if let Some(selection) = data
        .get_mut("contextSelection")
        .and_then(Value::as_object_mut)
    {
        for key in [
            "policy",
            "requestedSymbols",
            "maxRelatedSymbols",
            "maxFiles",
        ] {
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
        if data
            .get(key)
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
        {
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
    // The file hash stays: replace/hash binding after a symbol read depends on
    // it (Vue extraction and stale-edit protection). Anchors are display noise.
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
        data.insert(
            "range".into(),
            Value::Array(vec![start.unwrap(), end.unwrap()]),
        );
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
                pair_range(row_map);
                if let Some(context) = row_map.get_mut("context").and_then(Value::as_object_mut) {
                    pair_range(context);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;

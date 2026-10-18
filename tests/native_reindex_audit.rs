//! Explicit read-only audit through the native harness API, never raw SQL/MCP.
#![cfg(windows)]
use agent_harness::adapters;
use harness_domain::contracts::Request;
use serde_json::json;

#[test]
#[ignore = "reads the existing native core index; run only after an explicit reindex"]
fn existing_native_reindex_has_proven_call_and_no_confirmed_false_pairs() {
    assert_eq!(
        std::env::var("HARNESS_AUDIT_EXISTING_INDEX").as_deref(),
        Ok("1")
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let app = adapters::application(root).unwrap();
    let inspect = |symbol: &str,
                   target_path: &str,
                   source_name: Option<&str>,
                   source_path: &str| {
        let result = app.execute(Request::Explore(
            serde_json::from_value(json!({
                "query":format!("{symbol} {target_path}"), "symbol":symbol,
            "direction":"inbound", "maxDepth":1,"maxFiles":4,
                "maxChars":1000,"requireMatchingFiles":true,"syncIndex":false
            }))
            .unwrap(),
        ));
        assert_eq!(result["state"], "ok", "{result}");
        let graph = &result["data"]["graph"];
        let nodes = graph["nodes"].as_array().unwrap();
        // A homonym-selection cap can mark the overall graph truncated. Only
        // accept that case when every selected node was expanded and has no
        // edges/neighbors; the exact target must also be present below.
        let selected_empty_adjacencies = graph["edges"].as_array().unwrap().is_empty()
            && graph["traversal"]["expandedNodes"].as_u64() == Some(nodes.len() as u64)
            && graph["traversal"]["nodeDepths"]
                .as_object()
                .unwrap()
                .values()
                .all(|d| d["inbound"] == 0);
        assert!(
            graph["truncated"] == false || selected_empty_adjacencies,
            "incomplete adjacency proof: {graph}"
        );
        let target_ids: Vec<_> = nodes
            .iter()
            .filter(|n| n["name"] == symbol && n["filePath"] == target_path)
            .map(|n| n["id"].as_str().unwrap())
            .collect();
        assert!(
            !target_ids.is_empty(),
            "missing exact audit target: {result}"
        );
        let source_ids: Vec<_> = nodes
            .iter()
            .filter(|n| {
                n["filePath"] == source_path && source_name.is_none_or(|name| n["name"] == name)
            })
            .filter_map(|n| n["id"].as_str())
            .collect();
        let count = graph["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| {
                e["kind"] == "calls"
                    && e["source"]
                        .as_str()
                        .is_some_and(|id| source_ids.contains(&id))
                    && e["target"]
                        .as_str()
                        .is_some_and(|id| target_ids.contains(&id))
            })
            .count();
        println!(
            "{}",
            json!({"audit":"existing-native-index","target":symbol,"targetPath":target_path,"sourcePath":source_path,"matchingCalls":count,"truncated":graph["truncated"],"selectedEmptyAdjacenciesFullyExpanded":selected_empty_adjacencies,"writesPerformed":false})
        );
        count
    };
    assert_eq!(
        inspect(
            "sync_workspace_all",
            "src/adapters/index_store.rs",
            Some("synchronized"),
            "src/adapters/explore.rs"
        ),
        1
    );
    for (symbol, target, source) in [
        (
            "collect",
            "src/adapters/rust_symbols.rs",
            "src/adapters/process.rs",
        ),
        (
            "execute",
            "crates/application/src/lib.rs",
            "src/adapters/index_store.rs",
        ),
        (
            "run",
            "crates/application/src/lib.rs",
            "src/adapters/index_store.rs",
        ),
    ] {
        assert_eq!(
            inspect(symbol, target, None, source),
            0,
            "confirmed false pair returned"
        );
    }
}

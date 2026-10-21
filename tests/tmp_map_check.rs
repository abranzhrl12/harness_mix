use agent_harness::adapters;
use harness_domain::sql::SqlArgs;
use serde_json::json;

fn call(root: &std::path::Path, value: serde_json::Value) -> serde_json::Value {
    let args: SqlArgs = serde_json::from_value(value).unwrap();
    adapters::application(root).unwrap().execute(args.into_request())
}

#[test]
fn live_map() {
    let root = std::path::Path::new(r"E:\harness_mix");
    let status = call(
        root,
        json!({"action":"status","engine":"postgresql","connect":true}),
    );
    println!(
        "STATUS: {}",
        serde_json::to_string_pretty(&status).unwrap_or_default()
    );
    let map = call(root, json!({"action":"map","engine":"postgresql"}));
    println!(
        "MAP: {}",
        serde_json::to_string_pretty(&map).unwrap_or_default()
    );
}

use agent_harness::adapters;
use harness_domain::contracts::{Request, SearchArgs};
use serde_json::json;

#[test]
fn continuation_rejects_file_newly_excluded_by_policy() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.txt"), "needle\n".repeat(10001)).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let args: SearchArgs = serde_json::from_value(json!({"query":"needle", "limit":200})).unwrap();
    let first = app.execute(Request::Search(args));
    let mut next = first["data"]["nextCall"]["arguments"].clone();
    next["offset"] = json!(10000);
    std::fs::write(root.path().join(".harnessignore"), "a.txt\n").unwrap();
    let result = app.execute(Request::Search(serde_json::from_value(next).unwrap()));
    assert_eq!(result["state"], "error", "{result}");
    assert_eq!(result["diagnostics"][0]["code"], "STALE");
}

#[test]
fn continuation_reaches_the_first_match_in_the_next_directory() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("z")).unwrap();
    std::fs::write(root.path().join("a.txt"), "needle\n".repeat(10000)).unwrap();
    std::fs::write(root.path().join("z/b.txt"), "needle\n").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let args: SearchArgs = serde_json::from_value(json!({"query":"needle", "limit":200})).unwrap();
    let mut page = app.execute(Request::Search(args));
    let mut count = 0;
    loop {
        assert_eq!(page["state"], "ok", "{page}");
        count += page["data"]["matches"].as_array().unwrap().len();
        let Some(next) = page["data"]["nextCall"]["arguments"].as_object() else {
            break;
        };
        page = app.execute(Request::Search(
            serde_json::from_value(json!(next)).unwrap(),
        ));
    }
    assert_eq!(count, 10001);
    assert_eq!(page["data"]["scanComplete"], true);
}

#[test]
fn partial_text_scan_continues_past_match_budget() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.txt"), "needle\n".repeat(10001)).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let args: SearchArgs = serde_json::from_value(json!({"query":"needle", "limit":200})).unwrap();
    let mut page = app.execute(Request::Search(args));
    let mut lines = Vec::new();
    loop {
        assert_eq!(page["state"], "ok", "{page}");
        lines.extend(
            page["data"]["matches"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["line"].as_u64().unwrap()),
        );
        let Some(next) = page["data"]["nextCall"]["arguments"].as_object() else {
            break;
        };
        let args: SearchArgs = serde_json::from_value(json!(next)).unwrap();
        page = app.execute(Request::Search(args));
    }
    assert_eq!(lines.len(), 10001);
    assert_eq!(lines[10000], 10001);
    assert_eq!(page["data"]["scanComplete"], true);
}

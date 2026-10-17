use agent_harness::adapters;
use harness_domain::contracts::{Request, SearchArgs};
use serde_json::{Value, json};
fn scan(root: &std::path::Path, args: Value) -> Value {
    let app = adapters::application(root).unwrap();
    app.execute(Request::Search(
        serde_json::from_value::<SearchArgs>(args).unwrap(),
    ))
}
#[test]
fn counts_crlf_unterminated_empty_and_sorts_without_returning_source() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("three.txt"),
        "private source\r\nsecond\r\nthird",
    )
    .unwrap();
    std::fs::write(root.path().join("two.txt"), "a\nb\n").unwrap();
    std::fs::write(root.path().join("empty.txt"), "").unwrap();
    let result = scan(
        root.path(),
        json!({"kind":"file_size","minLines":1,"limit":1}),
    );
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["files"][0]["path"], "three.txt");
    assert_eq!(result["data"]["files"][0]["lines"], 3);
    assert_eq!(result["data"]["scanned"], 3);
    assert!(result["detail"]["ref"].is_string());
    assert!(!result.to_string().contains("private source"));
    let exact = scan(
        root.path(),
        json!({"kind":"file_size","scope":"two.txt","minLines":2}),
    );
    assert!(exact["data"]["files"].as_array().unwrap().is_empty());
    let empty = scan(
        root.path(),
        json!({"kind":"file_size","scope":"empty.txt","minLines":0}),
    );
    assert!(empty["data"]["files"].as_array().unwrap().is_empty());
}
#[test]
fn ignores_binary_generated_and_ignored_files_but_reports_oversize_as_partial() {
    let root = tempfile::tempdir().unwrap();
    for (name, bytes) in [
        ("good.txt", b"1\n2\n".as_slice()),
        ("binary.bin", b"\0\n"),
        ("invalid.txt", b"\xff\n"),
        ("generated.min.js", b"1\n2\n"),
        ("ignored.txt", b"1\n2\n"),
    ] {
        std::fs::write(root.path().join(name), bytes).unwrap();
    }
    std::fs::write(root.path().join(".gitignore"), "ignored.txt\n").unwrap();
    std::fs::create_dir(root.path().join("vendor")).unwrap();
    std::fs::write(root.path().join("vendor/not-counted.txt"), "1\n2\n").unwrap();
    let clean = scan(root.path(), json!({"kind":"file_size","minLines":1}));
    assert_eq!(
        clean["data"]["files"].as_array().unwrap().len(),
        1,
        "{clean}"
    );
    assert_eq!(clean["data"]["binarySkipped"], 2);
    assert_eq!(clean["data"]["complete"], true);
    std::fs::write(
        root.path().join("huge.txt"),
        vec![b'x'; 2 * 1024 * 1024 + 1],
    )
    .unwrap();
    let partial = scan(root.path(), json!({"kind":"file_size","minLines":1}));
    assert_eq!(partial["data"]["complete"], false, "{partial}");
    assert_eq!(partial["data"]["skipped"], 1);
}
#[test]
fn validates_query_threshold_and_path_without_relaxing_other_search_kinds() {
    let root = tempfile::tempdir().unwrap();
    for args in [
        json!({"kind":"file_size","query":"ignored"}),
        json!({"kind":"file_size","minLines":10000001}),
        json!({"kind":"text"}),
        json!({"kind":"text","query":"x","minLines":0}),
    ] {
        assert_eq!(
            scan(root.path(), args)["diagnostics"][0]["code"],
            "ARGUMENT"
        );
    }
    assert_eq!(
        scan(
            root.path(),
            json!({"kind":"file_size","scope":"../outside"})
        )["diagnostics"][0]["code"],
        "PATH"
    );
}

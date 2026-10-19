use harness_domain::contracts::{ReadArgs, ReplaceArgs, WriteArgs};
use serde_json::json;

#[test]
fn wider_read_accepts_aliases_but_rejects_duplicate_names_and_overflow() {
    let args: ReadArgs = serde_json::from_value(
        json!({"target_path":"a.rs","start":1,"lines":300,"sessionId":"session"}),
    )
    .unwrap();
    assert_eq!(args.path, "a.rs");
    assert_eq!(args.end, 300);
    assert_eq!(args.session.as_deref(), Some("session"));
    for value in [
        json!({"path":"a.rs","start":1,"lines":301}),
        json!({"path":"a.rs","start":usize::MAX,"lines":2}),
        json!({"path":"a.rs","target_path":"b.rs","start":1,"lines":1}),
        json!({"path":"a.rs","start":1,"lines":1,"session":"a","sessionId":"b"}),
    ] {
        assert!(serde_json::from_value::<ReadArgs>(value).is_err());
    }
}

#[test]
fn write_and_replace_accept_io_aliases_without_weakening_conflicts() {
    let write: WriteArgs =
        serde_json::from_value(json!({"target_path":"a.txt","content":"hi","sessionId":"s"}))
            .unwrap();
    assert_eq!(write.path, "a.txt");
    assert_eq!(write.session.as_deref(), Some("s"));
    let replace: ReplaceArgs = serde_json::from_value(
        json!({"target_path":"a.txt","oldText":"hi","newText":"bye","sessionId":"s"}),
    )
    .unwrap();
    assert_eq!(replace.path, "a.txt");
    assert!(replace.hash.is_empty());
    assert_eq!(replace.session.as_deref(), Some("s"));
    assert!(
        serde_json::from_value::<WriteArgs>(
            json!({"path":"a.txt","target_path":"b.txt","content":"hi"})
        )
        .is_err()
    );
}

#[test]
fn replace_base_hash_alias_preserves_hash_and_rejects_duplicate_preconditions() {
    for field in ["hash", "baseHash"] {
        for chunks in [false, true] {
            let mut value = json!({"path":"a.txt"});
            value[field] = json!("source-sha256");
            if chunks {
                value["chunks"] = json!([{"oldText":"hi","newText":"bye"}]);
            } else {
                value["oldText"] = json!("hi");
                value["newText"] = json!("bye");
            }
            let args: ReplaceArgs = serde_json::from_value(value).unwrap();
            assert_eq!(args.hash, "source-sha256");
            let canonical = serde_json::to_value(args).unwrap();
            assert_eq!(canonical["hash"], "source-sha256");
            assert!(canonical.get("baseHash").is_none());
        }
    }
    for alias_hash in ["source-sha256", "different-sha256"] {
        let error = serde_json::from_value::<ReplaceArgs>(json!({
            "path":"a.txt", "hash":"source-sha256", "baseHash":alias_hash,
            "oldText":"hi", "newText":"bye"
        }))
        .unwrap_err();
        assert!(error.to_string().contains("duplicate field `hash`"));
    }
}

#[test]
#[cfg(feature = "schema")]
fn replace_schema_advertises_base_hash_and_excludes_duplicate_preconditions() {
    let schema = serde_json::to_value(schemars::schema_for!(ReplaceArgs)).unwrap();
    assert!(schema["properties"].get("hash").is_some());
    assert_eq!(
        schema["properties"]["hash"],
        schema["properties"]["baseHash"]
    );
    assert!(
        schema["allOf"]
            .as_array()
            .unwrap()
            .iter()
            .any(|constraint| { constraint == &json!({"not":{"required":["hash","baseHash"]}}) })
    );
}

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

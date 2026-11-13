use harness_domain::contracts::{BatchTool, ReadArgs, ReplaceArgs, SearchArgs};
use serde_json::json;

#[test]
fn read_lines_is_a_count_and_canonical_output_uses_end() {
    let args: ReadArgs =
        serde_json::from_value(json!({"path":"x.rs","start":20,"lines":50})).unwrap();
    assert_eq!(args.end, 69);
    let value = serde_json::to_value(args).unwrap();
    assert_eq!(value["end"], 69);
    assert!(value.get("lines").is_none());
    for value in [
        json!({"path":"x","start":1,"lines":0}),
        json!({"path":"x","start":1,"lines":421}),
        json!({"path":"x","start":0,"lines":1}),
        json!({"path":"x","start":5,"end":4}),
        json!({"path":"x","start":1,"end":421}),
        json!({"path":"x","start":1,"end":5,"lines":5}),
        json!({"path":"x","start":usize::MAX,"lines":2}),
    ] {
        assert!(serde_json::from_value::<ReadArgs>(value).is_err());
    }

    let defaulted: ReadArgs = serde_json::from_value(json!({"path":"only-path.txt"})).unwrap();
    assert_eq!((defaulted.start, defaulted.end), (1, 120));
    let offset: ReadArgs = serde_json::from_value(json!({"path":"offset.txt","start":20})).unwrap();
    assert_eq!((offset.start, offset.end), (20, 139));
}

#[test]
fn read_accepts_line_aliases_and_preserves_unknown_field_rejection() {
    let camel: ReadArgs =
        serde_json::from_value(json!({"path":"a.rs","startLine":1,"endLine":10})).unwrap();
    assert_eq!(camel.path, "a.rs");
    assert_eq!((camel.start, camel.end), (1, 10));

    let snake: ReadArgs =
        serde_json::from_value(json!({"path":"a.rs","start_line":2,"end_line":12})).unwrap();
    assert_eq!((snake.start, snake.end), (2, 12));

    for invalid in [
        json!({"path":"a.rs","start":1,"startLine":2,"end":10}),
        json!({"path":"a.rs","start":1,"end":10,"endLine":12}),
        json!({"path":"a.rs","startLine":1,"endLine":10,"unknown":true}),
    ] {
        assert!(serde_json::from_value::<ReadArgs>(invalid).is_err());
    }
}

#[test]
fn aliases_preserve_validation_and_canonical_names() {
    let search: SearchArgs =
        serde_json::from_value(json!({"kind":"files","pattern":"creator"})).unwrap();
    assert_eq!(search.query, "creator");
    assert!(
        serde_json::from_value::<SearchArgs>(json!({"kind":"files","query":"a","pattern":"b"}))
            .is_err()
    );
    assert!(serde_json::from_value::<SearchArgs>(json!({"kind":"files","typo":"a"})).is_err());
    let read: BatchTool = serde_json::from_value(json!("read")).unwrap();
    assert_eq!(serde_json::to_value(read).unwrap(), "harness_read");
}

#[test]
fn replace_accepts_snake_case_and_case_variants_but_rejects_collisions() {
    for (old, new, allow) in [
        ("old_text", "new_text", "allow_unvalidated_text"),
        ("OLDTEXT", "NewText", "ALLOWUNVALIDATEDTEXT"),
    ] {
        let mut value = json!({"path":"x.css", "line":3});
        value[old] = json!("padding: 5px");
        value[new] = json!("padding: 8px");
        value[allow] = json!(true);
        let args: ReplaceArgs = serde_json::from_value(value).unwrap();
        assert_eq!(args.line, Some(3));
        assert_eq!(args.old_text, "padding: 5px");
        assert!(args.allow_unvalidated_text);
        let output = serde_json::to_value(args).unwrap();
        assert_eq!(output["newText"], "padding: 8px");
        assert_eq!(output["line"], 3);
        assert!(output.get("new_text").is_none());
    }
    let args: ReplaceArgs = serde_json::from_value(json!({
        "path":"x.css", "chunks":[{"OLD_TEXT":"a", "NEW_TEXT":"b"}]
    }))
    .unwrap();
    assert_eq!(args.chunks.unwrap()[0].new_text, "b");
    for value in [
        json!({"path":"x", "oldText":"a", "old_text":"b", "newText":"c"}),
        json!({"path":"x", "hash":"a", "BASE_HASH":"b", "oldText":"a", "newText":"c"}),
        json!({"path":"x", "oldText":"a", "newText":"b", "typo":true}),
        json!({"path":"x", "chunks":[{"oldText":"a", "OLD_TEXT":"b", "newText":"c"}]}),
    ] {
        assert!(serde_json::from_value::<ReplaceArgs>(value).is_err());
    }
}

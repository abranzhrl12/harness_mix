use harness_domain::contracts::{BatchTool, ReadArgs, SearchArgs};
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
        json!({"path":"x","start":1,"lines":101}),
        json!({"path":"x","start":1,"end":5,"lines":5}),
        json!({"path":"x","start":1}),
        json!({"path":"x","start":usize::MAX,"lines":2}),
    ] {
        assert!(serde_json::from_value::<ReadArgs>(value).is_err());
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

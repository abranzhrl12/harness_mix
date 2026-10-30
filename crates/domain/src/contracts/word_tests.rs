use super::*;
use serde_json::json;

fn args(path: &str) -> WordArgs {
    serde_json::from_value(json!({
        "path": path,
        "document": {
            "title": "Design",
            "blocks": [
                {"type":"heading","level":1,"text":"Overview"},
                {"type":"paragraph","text":"Workspace generated"}
            ]
        }
    }))
    .unwrap()
}

#[test]
fn validates_relative_docx_targets_and_rejects_traversal() {
    assert!(args("docs/design.docx").validate().is_ok());
    assert!(args("docs/design.DOCX").validate().is_ok());
    assert_eq!(args("../outside.docx").validate().unwrap_err().0, "PATH");
    assert_eq!(args("C:/outside.docx").validate().unwrap_err().0, "PATH");
    assert_eq!(args("docs/design.pdf").validate().unwrap_err().0, "ARGUMENT");
}

#[test]
fn rejects_unknown_document_and_block_fields() {
    let unknown_document = json!({
        "path":"docs/design.docx",
        "document":{"blocks":[],"unexpected":true}
    });
    assert!(serde_json::from_value::<WordArgs>(unknown_document).is_err());

    let unknown_block = json!({
        "path":"docs/design.docx",
        "document":{"blocks":[{"type":"paragraph","text":"x","unexpected":true}]}
    });
    assert!(serde_json::from_value::<WordArgs>(unknown_block).is_err());
}

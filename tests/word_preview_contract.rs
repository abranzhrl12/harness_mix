//! Contract tests for workspace-relative DOCX page previews.

use harness_domain::word::WordPreviewArgs;
use serde_json::json;

fn args(path: &str, page: u32) -> WordPreviewArgs {
    serde_json::from_value(json!({
        "path": path,
        "page": page,
        "session": "session-1"
    }))
    .expect("valid preview arguments deserialize")
}

#[test]
fn docx_preview_accepts_workspace_relative_docx_and_one_based_pages() {
    let preview = args("docs/guide.docx", 1);

    assert_eq!(preview.path, "docs/guide.docx");
    assert_eq!(preview.page, 1);
    assert_eq!(preview.session.as_deref(), Some("session-1"));
    assert!(preview.validate().is_ok());

    assert!(args("docs/guide.DOCX", 7).validate().is_ok());
}

#[test]
fn docx_preview_rejects_paths_that_escape_or_name_non_docx_files() {
    for path in [
        "../outside.docx",
        "docs/../../outside.docx",
        "/outside.docx",
        "\\outside.docx",
        "C:/outside.docx",
        "docs/guide.pdf",
    ] {
        assert!(args(path, 1).validate().is_err(), "accepted unsafe path: {path}");
    }
}

#[test]
fn docx_preview_rejects_page_zero_and_unknown_arguments() {
    assert!(args("docs/guide.docx", 0).validate().is_err());

    let unknown = serde_json::from_value::<WordPreviewArgs>(json!({
        "path": "docs/guide.docx",
        "page": 1,
        "overwrite": true
    }));
    assert!(unknown.is_err(), "unknown mutation-like fields must be rejected");
}

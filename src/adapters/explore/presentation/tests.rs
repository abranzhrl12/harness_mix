#[test]
fn numbers_current_source_without_changing_line_content() {
    let rendered = super::source("a.rs", 7, "fn a() {\r\n    b();\r\n}", false);
    assert!(rendered.contains("7\tfn a() {\n8\t    b();\n9\t}"));
    assert!(!rendered.contains("truncated"));
}

#[test]
fn source_cannot_close_its_fence_and_truncation_is_explicit() {
    let rendered = super::source("a.rs", 1, "```\nhello", true);
    assert!(rendered.contains("\n````\n1\t```"));
    assert!(rendered.contains("Source fragment truncated"));
}

use super::first_error_location;

#[test]
fn reports_the_first_js_syntax_error_line_and_column() {
    let source = "const ok=1;\r\nconst emoji = '😀'; const broken = ;\r\n";
    let (line, column) = first_error_location("sample.js", source).unwrap();
    assert_eq!(line, 2);
    // The astral emoji occupies two UTF-16 columns; CRLF starts a new line.
    assert_eq!(column, 36);
}

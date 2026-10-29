use super::worker::{document_language, encoded_character};
#[test]
fn jsx_documents_use_their_react_language_ids() {
    assert_eq!(document_language("app.tsx").unwrap(), "typescriptreact");
    assert_eq!(document_language("app.jsx").unwrap(), "javascriptreact");
}
#[test]
fn negotiated_encodings_count_astral_characters_and_reject_unknown_encoding_at_zero() {
    assert_eq!(encoded_character("a😀z", 0, 3, "utf-8").unwrap(), 5);
    assert_eq!(encoded_character("a😀z", 0, 3, "utf-16").unwrap(), 3);
    assert_eq!(encoded_character("a😀z", 0, 3, "utf-32").unwrap(), 2);
    assert_eq!(
        encoded_character("", 0, 0, "unknown").unwrap_err().0,
        "UNSUPPORTED"
    );
}

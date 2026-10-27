use super::serialized_size;
use serde_json::json;

#[test]
fn counts_exact_compact_json_bytes_without_allocating_a_serialized_row() {
    for value in [
        json!({"text":"emoji 😀 and escaped \n quote \""}),
        json!([1, true, null, {"nested":["x", "y"]}]),
    ] {
        assert_eq!(serialized_size(&value), value.to_string().len());
    }
}

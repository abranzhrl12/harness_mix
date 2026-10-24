use super::{clean_empty_metadata, response_with_options};
use serde_json::json;

#[test]
fn removes_null_results_from_read_search_and_batch_envelopes() {
    for tool in ["harness_read", "harness_search", "harness_batch"] {
        let mut response = json!({
            "scope": {"tool": tool},
            "data": {
                "results": null,
                "_detail": {"results": null},
                "optional": null
            }
        });

        clean_empty_metadata(&mut response);

        assert_eq!(
            response,
            json!({
                "scope": {"tool": tool},
                "data": {"optional": null}
            })
        );
    }

    let mut payload = json!({
        "scope": {"tool": "harness_sql"},
        "data": {"rows": [{"results": null}], "_detail": {"results": null}}
    });
    let expected = payload.clone();
    clean_empty_metadata(&mut payload);
    assert_eq!(payload, expected);

    let mut populated = json!({
        "scope": {"tool": "harness_search"},
        "data": {
            "results": [{"match": 1}],
            "_detail": {"results": [{"match": 2}], "cursor": "next"}
        }
    });
    let expected = populated.clone();
    clean_empty_metadata(&mut populated);
    assert_eq!(populated, expected);
}

#[test]
fn browser_screenshot_becomes_image_content_without_duplicate_base64() {
    let result = response_with_options(
        json!({
            "state": "ok",
            "scope": {"tool": "harness_browser"},
            "data": {
                "action": "screenshot",
                "mimeType": "image/png",
                "dataBase64": "aGVsbG8=",
                "width": 12,
                "height": 8
            }
        }),
        false,
    );

    assert!(matches!(
        result.content.as_slice(),
        [rmcp::model::ContentBlock::Image(image)]
            if image.data == "aGVsbG8=" && image.mime_type == "image/png"
    ));
    let wire = serde_json::to_value(result).unwrap();
    assert!(
        wire["structuredContent"]["data"]
            .get("dataBase64")
            .is_none()
    );
    assert_eq!(wire["structuredContent"]["data"]["width"], 12);
    assert_eq!(wire["structuredContent"]["data"]["height"], 8);
}

//! Integration tests for native SVG inspection and preview.

use agent_harness::adapters;
use harness_domain::{
    contracts::{BatchArgs, Request},
    svg::{SvgArgs, SvgMode},
};
use serde_json::json;
use std::fs;
use tempfile::tempdir;

const VALID_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="80" viewBox="0 0 100 80">
  <defs>
    <linearGradient id="grad1">
      <stop offset="0%" stop-color="red" />
      <stop offset="100%" stop-color="blue" />
    </linearGradient>
  </defs>
  <rect id="box" width="100" height="80" fill="url(#grad1)" />
  <circle cx="50" cy="40" r="30" fill="green" />
  <text x="10" y="20">Hello SVG</text>
</svg>"##;

const UNSAFE_SVG_EXTERNAL_URL: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
  <rect width="100" height="100" fill="url(https://malicious.com/test.png)" />
</svg>"##;

const UNSAFE_SVG_SCRIPT_EVENT: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
  <circle onclick="alert(1)" cx="50" cy="50" r="40" />
</svg>"##;

const UNSAFE_SVG_CYCLE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
  <g id="a">
    <use href="#b" />
  </g>
  <g id="b">
    <use href="#a" />
  </g>
</svg>"##;

#[test]
fn test_svg_inspect_mode() {
    let dir = tempdir().unwrap();
    let app = adapters::application(dir.path()).unwrap();

    fs::write(dir.path().join("icon.svg"), VALID_SVG).unwrap();

    let req = Request::Svg(SvgArgs {
        path: "icon.svg".to_string(),
        mode: Some(SvgMode::Inspect),
        max_size: None,
        session: None,
        external_scope: None,
    });

    let res = app.execute(req);
    assert_eq!(res["state"], "ok", "{res}");
    let data = &res["data"];
    assert_eq!(data["path"], "icon.svg");
    assert_eq!(data["mode"], "inspect");
    assert_eq!(data["profile"], "static-svg-subset");
    assert_eq!(data["dimensions"]["width"], "100");
    assert_eq!(data["dimensions"]["height"], "80");
    assert_eq!(data["dimensions"]["viewBox"], "0 0 100 80");

    let counts = &data["counts"];
    assert_eq!(counts["svg"], 1);
    assert_eq!(counts["defs"], 1);
    assert_eq!(counts["linearGradient"], 1);
    assert_eq!(counts["stop"], 2);
    assert_eq!(counts["rect"], 1);
    assert_eq!(counts["circle"], 1);
    assert_eq!(counts["text"], 1);

    let items = data["items"].as_array().unwrap();
    assert!(!items.is_empty());
    // Find text node
    let text_item = items.iter().find(|it| it["tag"] == "text").unwrap();
    assert_eq!(text_item["text"], "Hello SVG");

    // Check url(#grad1) internal reference extraction
    let rect_item = items.iter().find(|it| it["tag"] == "rect").unwrap();
    let refs = rect_item["references"].as_array().unwrap();
    assert_eq!(refs[0], "grad1");
}

#[test]
fn test_svg_preview_mode() {
    let dir = tempdir().unwrap();
    let app = adapters::application(dir.path()).unwrap();

    fs::write(dir.path().join("icon.svg"), VALID_SVG).unwrap();

    let req = Request::Svg(SvgArgs {
        path: "icon.svg".to_string(),
        mode: Some(SvgMode::Preview),
        max_size: Some(512),
        session: None,
        external_scope: None,
    });

    let res = app.execute(req);
    assert_eq!(res["state"], "ok", "{res}");
    let data = &res["data"];
    assert_eq!(data["mode"], "preview");
    assert_eq!(data["width"], 100);
    assert_eq!(data["height"], 80);
    assert!(data["fonts"].as_str().unwrap().contains("system fonts"));
}

#[test]
fn test_svg_security_rejections() {
    let dir = tempdir().unwrap();
    let app = adapters::application(dir.path()).unwrap();

    // 1. External URL
    fs::write(dir.path().join("external.svg"), UNSAFE_SVG_EXTERNAL_URL).unwrap();
    let res = app.execute(Request::Svg(SvgArgs {
        path: "external.svg".to_string(),
        mode: None,
        max_size: None,
        session: None,
        external_scope: None,
    }));
    assert_eq!(res["state"], "error");
    assert_eq!(res["diagnostics"][0]["code"], "SVG_UNSAFE");

    // 2. Active script / event handler
    fs::write(dir.path().join("script.svg"), UNSAFE_SVG_SCRIPT_EVENT).unwrap();
    let res = app.execute(Request::Svg(SvgArgs {
        path: "script.svg".to_string(),
        mode: None,
        max_size: None,
        session: None,
        external_scope: None,
    }));
    assert_eq!(res["state"], "error");
    assert_eq!(res["diagnostics"][0]["code"], "SVG_UNSAFE");

    // 3. Cyclic reference
    fs::write(dir.path().join("cycle.svg"), UNSAFE_SVG_CYCLE).unwrap();
    let res = app.execute(Request::Svg(SvgArgs {
        path: "cycle.svg".to_string(),
        mode: None,
        max_size: None,
        session: None,
        external_scope: None,
    }));
    assert_eq!(res["state"], "error");
    assert_eq!(res["diagnostics"][0]["code"], "SVG_UNSAFE");

    // 4. Invalid extension
    fs::write(dir.path().join("not_svg.txt"), VALID_SVG).unwrap();
    let res = app.execute(Request::Svg(SvgArgs {
        path: "not_svg.txt".to_string(),
        mode: None,
        max_size: None,
        session: None,
        external_scope: None,
    }));
    assert_eq!(res["state"], "error");
    assert_eq!(res["diagnostics"][0]["code"], "PATH");
}

#[test]
fn test_svg_batch_execution() {
    let dir = tempdir().unwrap();
    let app = adapters::application(dir.path()).unwrap();

    fs::write(dir.path().join("icon.svg"), VALID_SVG).unwrap();

    let batch_req = Request::Batch(serde_json::from_value::<BatchArgs>(json!({
        "jobs": [
            {
                "tool": "harness_svg",
                "arguments": {
                    "path": "icon.svg",
                    "mode": "inspect"
                }
            },
            {
                "tool": "harness_svg",
                "arguments": {
                    "path": "icon.svg",
                    "mode": "preview",
                    "maxSize": 256
                }
            }
        ]
    })).unwrap());

    let batch_res = app.execute(batch_req);
    assert_eq!(batch_res["state"], "ok", "{batch_res}");
    let results = batch_res["data"]["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["state"], "ok");
    assert_eq!(results[0]["data"]["mode"], "inspect");
    assert_eq!(results[1]["state"], "ok");
    assert_eq!(results[1]["data"]["mode"], "preview");
}

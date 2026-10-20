use agent_harness::adapters;
use harness_domain::{
    contracts::{OutlineArgs, ReadSymbolArgs, Request},
    hash,
};
use serde_json::Value;

fn outline(app: &agent_harness::adapters::NativeApplication, path: &str) -> Value {
    app.execute(Request::Outline(OutlineArgs {
        path: path.into(),
        query: None,
        max_chars: Some(16000),
        session: None,
        external_scope: None,
    }))
}

fn read(
    app: &agent_harness::adapters::NativeApplication,
    path: &str,
    name: &str,
    id: Option<&str>,
) -> Value {
    app.execute(Request::ReadSymbol(ReadSymbolArgs {
        path: path.into(),
        symbol: name.into(),
        container: None,
        symbol_id: id.map(Into::into),
        view: None,
        session: None,
        external_scope: None,
    }))
}

#[test]
fn css_rule_reads_preserve_exact_source_hash_and_ignore_strings_and_comments() {
    let root = tempfile::tempdir().unwrap();
    let rule = "#orden-servicio-tabs .fast-tabs__item {\r\n  padding: 5px; content: \"}🙂{\"; /* } */\r\n}";
    let source = format!(
        "/* header {{ */\r\n{rule}\r\n@media (width > 10px) {{\n.card[data-x=\"{{\"] {{ color: red; }}\n}}\n"
    );
    std::fs::write(root.path().join("main.css"), &source).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = read(
        &app,
        "main.css",
        "#orden-servicio-tabs .fast-tabs__item",
        None,
    );
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["text"], rule);
    assert_eq!(result["data"]["hash"], hash(source.as_bytes()));
    assert_eq!(result["data"]["symbol"]["start"], 2);
    let out = outline(&app, "main.css");
    let items = out["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 3, "{out}");
    assert!(
        items.iter().any(
            |s| s["name"] == ".card[data-x=\"{\"]" && s["container"] == "@media (width > 10px)"
        )
    );
    let id = result["data"]["symbol"]["symbolId"].as_str().unwrap();
    std::fs::write(root.path().join("main.css"), format!("{source}\n")).unwrap();
    assert_eq!(
        read(
            &app,
            "main.css",
            "#orden-servicio-tabs .fast-tabs__item",
            Some(id)
        )["diagnostics"][0]["code"],
        "STALE_SYMBOL"
    );
}

#[test]
fn html_and_razor_include_multiple_style_blocks_with_original_offsets() {
    let root = tempfile::tempdir().unwrap();
    let source = "<main>🙂</main>\n<STYLE media=\"screen\">\n.button { padding: 5px; }\n</STYLE>\n<style>.other { color: blue; }</style>\n<!-- <style>.fake {}</style> -->\n<script>const text = '<style>.fake {}</style>';</script>\n<div data-text=\"<style>.fake {}</style>\"></div>";
    for path in ["page.html", "page.cshtml"] {
        std::fs::write(root.path().join(path), source).unwrap();
    }
    let app = adapters::application(root.path()).unwrap();
    for path in ["page.html", "page.cshtml"] {
        let out = outline(&app, path);
        assert_eq!(out["state"], "ok", "{out}");
        assert!(
            out["data"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["name"] == ".other")
        );
        assert!(
            !out["data"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["name"] == ".fake")
        );
        let result = read(&app, path, ".button", None);
        assert_eq!(result["data"]["text"], ".button { padding: 5px; }");
        assert_eq!(result["data"]["hash"], hash(source.as_bytes()));
        assert_eq!(result["data"]["symbol"]["start"], 3);
    }
}

#[test]
fn repeated_selectors_require_current_symbol_id() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("main.css"),
        ".a { color:red; } .a { color:blue; }",
    )
    .unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = read(&app, "main.css", ".a", None);
    assert_eq!(result["diagnostics"][0]["code"], "AMBIGUOUS_SYMBOL");
    let out = outline(&app, "main.css");
    let id = out["data"]["items"][1]["symbolId"].as_str().unwrap();
    assert_eq!(
        read(&app, "main.css", ".a", Some(id))["data"]["text"],
        ".a { color:blue; }"
    );
}

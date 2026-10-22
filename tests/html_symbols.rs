use agent_harness::adapters;
use harness_domain::{
    contracts::{OutlineArgs, ReadSymbolArgs, ReplaceArgs, Request},
    hash,
};
use serde_json::{Value, json};
use std::fs;

fn outline(app: &adapters::NativeApplication, path: &str) -> Value {
    app.execute(Request::Outline(
        serde_json::from_value::<OutlineArgs>(json!({"path":path,"maxChars":16000})).unwrap(),
    ))
}
fn read(app: &adapters::NativeApplication, selector: &str, id: Option<&str>) -> Value {
    app.execute(Request::ReadSymbol(
        serde_json::from_value::<ReadSymbolArgs>(json!({
            "path":"page.html","symbol":selector,"symbolId":id,"view":"full"
        }))
        .unwrap(),
    ))
}

#[test]
fn html_outline_exposes_elements_attributes_parents_and_exact_unicode_source() {
    let root = tempfile::tempdir().unwrap();
    let button =
        "<button id='guardar' class=\"action primary\" data-hint=\"a > b\">Guardar 🙂</button>";
    let source = format!(
        "\u{feff}<!-- cabecera -->\r\n<main id=\"root\">\r\n  {button}\r\n  <input disabled>\r\n</main>\r\n"
    );
    fs::write(root.path().join("page.html"), &source).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let out = outline(&app, "page.html");
    assert_eq!(out["state"], "ok", "{out}");
    let elements = out["data"]["items"].as_array().unwrap();
    let button_item = elements
        .iter()
        .find(|e| e["name"] == "html:button#guardar")
        .unwrap();
    assert_eq!(button_item["container"], "html:main#root");
    assert_eq!(button_item["start"], 3);
    assert!(
        button_item["attributes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "data-hint=\"a > b\"")
    );
    assert_eq!(out["data"]["structure"]["parser"], "tl");
    let result = read(&app, "html:#guardar", None);
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["text"], button);
    assert_eq!(result["data"]["hash"], hash(source.as_bytes()));
    let selected = read(&app, "html:main button.action", None);
    assert_eq!(selected["data"]["text"], button, "{selected}");
    assert_eq!(
        read(&app, "html:button[data-hint]", None)["data"]["text"],
        button
    );
    assert_eq!(
        read(&app, "html:input", None)["data"]["text"],
        "<input disabled>"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("page.html")).unwrap(),
        source
    );
}

#[test]
fn angular_attributes_control_flow_and_self_closing_components_keep_original_bytes() {
    let root = tempfile::tempdir().unwrap();
    let button = "<button id=\"save\" [disabled]=\"busy()\" (click)=\"save()\" [(ngModel)]=\"value\" *ngIf=\"ready\">{{ label }}</button>";
    let source = format!(
        "@if (ready()) {{\r\n{button}\r\n}} @else {{ <app-empty /> }}\r\n@for (item of items(); track item.id) {{ <app-row [item]=\"item\" /> }}\r\n"
    );
    fs::write(root.path().join("page.html"), &source).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = read(&app, "html:#save", None);
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["text"], button);
    let attributes = result["data"]["symbol"]["attributes"].as_array().unwrap();
    assert!(
        attributes.iter().any(|a| a == "[disabled]=\"busy()\""),
        "{attributes:?}"
    );
    assert!(attributes.iter().any(|a| a == "(click)=\"save()\""));
    assert!(attributes.iter().any(|a| a == "[(ngModel)]=\"value\""));
    assert_eq!(
        read(&app, "html:app-empty", None)["data"]["text"],
        "<app-empty />"
    );
    assert_eq!(
        read(&app, "html:app-row", None)["data"]["text"],
        "<app-row [item]=\"item\" />"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("page.html")).unwrap(),
        source
    );
}

#[test]
fn duplicate_elements_require_symbol_id_and_stale_ids_cannot_target_a_changed_file() {
    let root = tempfile::tempdir().unwrap();
    let source =
        "<div><button class='action'>One</button><button class='action'>Two</button></div>";
    fs::write(root.path().join("page.html"), source).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = read(&app, "html:.action", None);
    assert_eq!(
        result["diagnostics"][0]["code"], "AMBIGUOUS_SYMBOL",
        "{result}"
    );
    let id = result["data"]["candidates"][1]["symbolId"]
        .as_str()
        .unwrap();
    assert_eq!(
        read(&app, "html:.action", Some(id))["data"]["text"],
        "<button class='action'>Two</button>"
    );
    fs::write(root.path().join("page.html"), format!("{source}\n")).unwrap();
    assert_eq!(
        read(&app, "html:.action", Some(id))["diagnostics"][0]["code"],
        "STALE_SYMBOL"
    );
}

#[test]
fn comments_and_raw_script_style_content_do_not_create_fake_html_elements() {
    let root = tempfile::tempdir().unwrap();
    let source = "<!-- <button id='comment'> -->\n<script>const x = '<button id=\"script\">fake</button>';</script>\n<style>.real { content: '<button id=\"css\">'; }</style>\n<button id='real'>real</button>";
    fs::write(root.path().join("page.html"), source).unwrap();
    let app = adapters::application(root.path()).unwrap();
    for selector in ["html:#comment", "html:#script", "html:#css"] {
        assert_eq!(read(&app, selector, None)["state"], "error", "{selector}");
    }
    assert_eq!(
        read(&app, "html:#real", None)["data"]["text"],
        "<button id='real'>real</button>"
    );
    assert_eq!(
        read(&app, ".real", None)["data"]["text"],
        ".real { content: '<button id=\"css\">'; }"
    );
}

#[test]
#[cfg(windows)]
fn structural_read_is_edit_ready_without_reserializing_other_html() {
    let root = tempfile::tempdir().unwrap();
    let target = "<button id = 'save'  data-x=\"🙂\">Old</button>";
    let source = format!("<!doctype html>\r\n<div>\r\n{target}\r\n<button>Old</button>\r\n</div>");
    fs::write(root.path().join("page.html"), &source).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = read(&app, "html:#save", None);
    assert_eq!(result["state"], "ok", "{result}");
    let original = result["data"]["text"].as_str().unwrap();
    assert_eq!(original, target);
    let args: ReplaceArgs =
        serde_json::from_value(json!({"path":"page.html","hash":result["data"]["hash"],
            "oldText":original,"newText":original.replace("Old", "New")
        }))
        .unwrap();
    let edited = app.execute(Request::Replace(args));
    assert_eq!(edited["state"], "ok", "{edited}");
    assert_eq!(
        fs::read_to_string(root.path().join("page.html")).unwrap(),
        source.replacen("Old", "New", 1)
    );
}

#[test]
fn angular_comparisons_and_tag_strings_remain_opaque_to_html_navigation() {
    let root = tempfile::tempdir().unwrap();
    let button = "<button id='safe'>{{ a<b ? '<ghost>' : label }}</button>";
    let source = format!(
        "{{{{ '<script>' }}}}\n@if (a<b) {{ {button} }}\n<button id='after'>After</button>"
    );
    fs::write(root.path().join("page.html"), &source).unwrap();
    let app = adapters::application(root.path()).unwrap();
    assert_eq!(read(&app, "html:#safe", None)["data"]["text"], button);
    assert_eq!(
        read(&app, "html:#after", None)["data"]["text"],
        "<button id='after'>After</button>"
    );
    assert_eq!(read(&app, "html:ghost", None)["state"], "error");
    assert_eq!(read(&app, "html:script", None)["state"], "error");
    assert_eq!(
        fs::read_to_string(root.path().join("page.html")).unwrap(),
        source
    );
}

#[test]
fn selector_errors_are_explicit_and_large_sibling_lists_are_bounded() {
    let root = tempfile::tempdir().unwrap();
    let source = format!(
        "<main>{}<button id='last'>Last</button></main>",
        "<button class='other'>Other</button>".repeat(2500)
    );
    fs::write(root.path().join("page.html"), &source).unwrap();
    let app = adapters::application(root.path()).unwrap();
    assert_eq!(
        read(&app, "html:main #last", None)["data"]["text"],
        "<button id='last'>Last</button>"
    );
    for selector in [
        "html:",
        "html:button > span",
        "html:button:hover",
        "html:[id='last'🙂]",
        "html:[id=]",
        "html:[id=\"a]",
    ] {
        let result = read(&app, selector, None);
        assert_eq!(
            result["diagnostics"][0]["code"], "ARGUMENT",
            "{selector}: {result}"
        );
    }
    fs::write(root.path().join("page.html"), "<i></i>".repeat(4001)).unwrap();
    assert_eq!(
        read(&app, "html:i", None)["diagnostics"][0]["code"],
        "PARSER_LIMIT"
    );
}

#[test]
fn unfinished_markup_never_panics_or_changes_the_source() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    for source in [
        "<",
        "<div",
        "<div><span>",
        "<input x='unfinished",
        "<script>const s = '<div>';",
        "{{ a<",
    ] {
        fs::write(root.path().join("page.html"), source).unwrap();
        let result = outline(&app, "page.html");
        assert!(
            matches!(result["state"].as_str(), Some("ok" | "error")),
            "{result}"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("page.html")).unwrap(),
            source
        );
    }
}

use agent_harness::adapters::{self, NativeApplication};
use harness_domain::{
    contracts::{DetailArgs, OutlineArgs, ReadSymbolArgs, Request, SymbolView, SymbolsArgs},
    hash,
};
use serde_json::Value;

#[test]
fn modest_multiline_symbol_returns_exact_body_in_one_call() {
    let root = tempfile::tempdir().unwrap();
    let text = format!(
        "fn readable() {{\n{}}}",
        "    let _ = \"🙂\";\n".repeat(120)
    );
    std::fs::write(root.path().join("sample.rs"), &text).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = read(&app, "readable", None, None, false);
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["text"], text);
    assert!(result["detail"].is_null());
    assert_eq!(result["data"]["hash"], hash(text.as_bytes()));
}

fn outline(app: &NativeApplication, query: Option<&str>, max: usize) -> Value {
    app.execute(
        SymbolsArgs::Outline(OutlineArgs {
            external_scope: None,
            path: "sample.rs".into(),
            query: query.map(Into::into),
            max_chars: Some(max),
            filter_kind: None,
            view: None,
            session: None,
        })
        .into_request(),
    )
}
fn read(
    app: &NativeApplication,
    name: &str,
    container: Option<&str>,
    id: Option<&str>,
    signature: bool,
) -> Value {
    app.execute(
        SymbolsArgs::Read(ReadSymbolArgs {
            external_scope: None,
            path: "sample.rs".into(),
            symbol: name.into(),
            container: container.map(Into::into),
            symbol_id: id.map(Into::into),
            view: signature.then_some(SymbolView::Signature),
            session: None,
        })
        .into_request(),
    )
}

#[test]
fn exact_spans_containers_signatures_and_stale_ids_survive_unicode_bom_and_shebang() {
    let root = tempfile::tempdir().unwrap();
    let text = "\u{feff}#!/usr/bin/env rust-script\r\n// 🙂 Unicode\r\nmod inner {\r\n    /// A documented type\r\n    pub struct Foo { x: usize }\r\n    impl Foo {\r\n        /// Method documentation\r\n        pub fn ping(&self) -> &str { \"🙂\" }\r\n    }\r\n    pub fn ping() {}\r\n}\r\n";
    std::fs::write(root.path().join("sample.rs"), text).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let output = outline(&app, None, 16000);
    assert_eq!(output["state"], "ok", "{output}");
    let items = output["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 5, "{items:?}");
    assert!(
        items
            .iter()
            .any(|s| s["container"] == "inner::impl Foo" && s["name"] == "ping")
    );
    let symbol = read(&app, "ping", Some("inner::impl Foo"), None, false);
    assert_eq!(symbol["state"], "ok", "{symbol}");
    let body = symbol["data"]["text"].as_str().unwrap();
    assert!(text.contains(body));
    assert!(body.contains("pub fn ping(&self) -> &str { \"🙂\" }"));
    assert_eq!(symbol["data"]["hash"], hash(text.as_bytes()));
    assert_eq!(symbol["data"]["symbol"]["doc"], " Method documentation");
    let id = symbol["data"]["symbol"]["symbolId"].as_str().unwrap();
    let signature = read(&app, "ping", Some("inner::impl Foo"), Some(id), true);
    assert_eq!(signature["state"], "ok", "{signature}");
    assert_eq!(signature["data"]["editReady"], false);
    assert!(signature["data"].get("text").is_none());
    assert!(
        signature["data"]["symbol"]["signature"]
            .as_str()
            .unwrap()
            .starts_with("pub fn ping")
    );
    assert_eq!(
        read(&app, "ping", None, None, false)["diagnostics"][0]["code"],
        "AMBIGUOUS_SYMBOL"
    );
    std::fs::write(
        root.path().join("sample.rs"),
        format!("{text}// changed\r\n"),
    )
    .unwrap();
    assert_eq!(
        read(&app, "ping", Some("inner::impl Foo"), Some(id), false)["diagnostics"][0]["code"],
        "STALE_SYMBOL"
    );
}

#[test]
fn large_symbols_and_budgeted_outlines_are_recoverable_by_detail() {
    let root = tempfile::tempdir().unwrap();
    let text = format!(
        "fn large() {{\n{}\n}}\n{}",
        "    let _ = \"🙂\";\n".repeat(120) + &format!("    // {}\n", "é".repeat(8000)),
        (0..60)
            .map(|i| format!("fn method_{i}() {{}}\n"))
            .collect::<String>()
    );
    std::fs::write(root.path().join("sample.rs"), &text).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let selected = outline(&app, Some("method_59"), 1000);
    assert_eq!(selected["state"], "ok", "{selected}");
    assert_eq!(selected["data"]["items"][0]["name"], "method_59");
    assert!(selected["omitted"]["matches"].as_u64().unwrap() > 0);
    assert!(selected["detail"]["ref"].is_string());
    let large = read(&app, "large", None, None, false);
    assert_eq!(large["state"], "ok", "{large}");
    assert!(large["data"]["text"].is_null());
    assert_eq!(large["omitted"]["truncated"], true);
    let detail = app.execute(Request::Detail(DetailArgs {
        reference: large["detail"]["ref"].as_str().unwrap().into(),
        offset: Some(0),
        max_chars: Some(16000),
        session: None,
    }));
    let page = detail["data"]["text"].as_str().unwrap();
    let value: Value = serde_json::from_str(page).unwrap();
    assert!(value["text"].as_str().unwrap().starts_with("fn large()"));
    assert!(text.contains(value["text"].as_str().unwrap()));
}

#[test]
fn bounds_selectors_and_parser_work_and_rejects_other_languages() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("sample.rs"), "fn sample() {}").unwrap();
    let app = adapters::application(root.path()).unwrap();
    assert_eq!(
        outline(&app, None, 999)["diagnostics"][0]["code"],
        "ARGUMENT"
    );
    assert_eq!(
        read(&app, "sample", None, Some("invalid"), false)["diagnostics"][0]["code"],
        "ARGUMENT"
    );
    assert_eq!(
        read(
            &app,
            "sample",
            None,
            Some("000000000000000000000000"),
            false
        )["diagnostics"][0]["code"],
        "STALE_SYMBOL"
    );
    let missing = app.execute(Request::Outline(OutlineArgs {
        external_scope: None,
        path: "sample.rs".into(),
        query: None,
        max_chars: None,
        filter_kind: None,
        view: None,
        session: Some("missing".into()),
    }));
    assert_eq!(missing["diagnostics"][0]["code"], "SESSION");
    let unsupported = app.execute(Request::Outline(OutlineArgs {
        external_scope: None,
        path: "sample.py".into(),
        query: None,
        max_chars: None,
        filter_kind: None,
        view: None,
        session: None,
    }));
    assert_eq!(unsupported["diagnostics"][0]["code"], "PARSER_UNAVAILABLE");
    std::fs::write(
        root.path().join("sample.rs"),
        (0..1001)
            .map(|i| format!("fn f_{i}() {{}}\n"))
            .collect::<String>(),
    )
    .unwrap();
    assert_eq!(
        outline(&app, None, 8000)["diagnostics"][0]["code"],
        "PARSER_LIMIT"
    );
    std::fs::write(root.path().join("sample.rs"), " ".repeat(256 * 1024 + 1)).unwrap();
    assert_eq!(
        outline(&app, None, 8000)["diagnostics"][0]["code"],
        "PARSER_LIMIT"
    );
}

#[test]
fn declaration_kinds_trait_members_and_repeated_parsing_are_stable() {
    let root = tempfile::tempdir().unwrap();
    let text = "const C: i32=1; static S: i32=2; type Alias=i32; enum E { A } union U { x:i32 } trait T { fn run(&self); type A; const X:i32; } impl T for U { fn run(&self){} type A=i32; const X:i32=0; } macro_rules! make { () => {} }";
    std::fs::write(root.path().join("sample.rs"), text).unwrap();
    let app = adapters::application(root.path()).unwrap();
    for _ in 0..5 {
        let out = outline(&app, None, 16000);
        assert_eq!(out["state"], "ok", "{out}");
        assert!(
            out["data"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["name"] == "run" && s["container"] == "impl T for U")
        );
        let symbol = read(&app, "run", Some("T"), None, false);
        assert_eq!(symbol["data"]["text"], "fn run(&self);");
    }
    std::fs::write(root.path().join("sample.rs"), "fn invalid( {").unwrap();
    assert_eq!(
        outline(&app, None, 16000)["diagnostics"][0]["code"],
        "SYNTAX_ERROR"
    );
}

//! Integration tests for JS and TS outline and read_symbol navigation.

use agent_harness::adapters::{self, NativeApplication};
use harness_domain::contracts::{OutlineArgs, ReadSymbolArgs, SymbolView, SymbolsArgs};
use serde_json::Value;

#[test]
fn source_format_classification_preserves_ecmascript_dialects_and_protocol_language() {
    let root = tempfile::tempdir().unwrap();
    let cases = [
        (
            "module.JS",
            "export function value() { return 1; }",
            "javascript",
        ),
        (
            "module.TS",
            "export function value(): number { return 1; }",
            "typescript",
        ),
        (
            "view.JSX",
            "export function value() { return <div />; }",
            "javascript",
        ),
        (
            "view.TSX",
            "export function value(): JSX.Element { return <div />; }",
            "typescript",
        ),
    ];
    for (path, source, _) in cases {
        std::fs::write(root.path().join(path), source).unwrap();
    }
    let app = adapters::application(root.path()).unwrap();
    for (path, source, language) in cases {
        let result = outline(&app, path, None);
        assert_eq!(result["state"], "ok", "{result}");
        assert_eq!(result["data"]["language"], language, "{path}");
        let symbol = result["data"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == "value")
            .unwrap();
        let read = read_symbol(
            &app,
            path,
            "value",
            None,
            symbol["symbolId"].as_str(),
            false,
        );
        assert_eq!(read["state"], "ok", "{read}");
        assert!(source.contains(read["data"]["text"].as_str().unwrap()));
    }
}

fn outline(app: &NativeApplication, path: &str, query: Option<&str>) -> Value {
    app.execute(
        SymbolsArgs::Outline(OutlineArgs {
            external_scope: None,
            path: path.into(),
            query: query.map(Into::into),
            max_chars: Some(16000),
            session: None,
        })
        .into_request(),
    )
}

fn read_symbol(
    app: &NativeApplication,
    path: &str,
    name: &str,
    container: Option<&str>,
    id: Option<&str>,
    signature: bool,
) -> Value {
    app.execute(
        SymbolsArgs::Read(ReadSymbolArgs {
            external_scope: None,
            path: path.into(),
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
fn test_js_outline_and_read_symbol() {
    let root = tempfile::tempdir().unwrap();
    let js_code = r#"/**
 * Calculate user summary
 */
function calculateSummary(user, factor) {
    const total = factor * 10;
    return { user, total };
}

class UserService {
    /**
     * Get user by ID
     */
    getUser(id) {
        return { id, name: "Alice" };
    }
}

const formatName = (first, last) => `${first} ${last}`;
"#;

    std::fs::write(root.path().join("service.js"), js_code).unwrap();
    let app = adapters::application(root.path()).unwrap();

    let out = outline(&app, "service.js", None);
    assert_eq!(out["state"], "ok");
    assert_eq!(out["data"]["language"], "javascript");
    assert_eq!(out["omitted"], serde_json::json!({}));

    let items = out["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 4); // calculateSummary, UserService, getUser, formatName

    let func_item = items
        .iter()
        .find(|s| s["name"] == "calculateSummary")
        .unwrap();
    assert_eq!(func_item["kind"], "function_declaration");
    assert!(
        func_item["doc"]
            .as_str()
            .unwrap()
            .contains("Calculate user summary")
    );

    let method_item = items.iter().find(|s| s["name"] == "getUser").unwrap();
    assert_eq!(method_item["container"], "UserService");
    assert_eq!(method_item["kind"], "method_definition");

    let arrow_item = items.iter().find(|s| s["name"] == "formatName").unwrap();
    assert_eq!(arrow_item["kind"], "function_definition");

    // Read full symbol
    let read_res = read_symbol(&app, "service.js", "calculateSummary", None, None, false);
    assert_eq!(read_res["state"], "ok");
    let text = read_res["data"]["text"].as_str().unwrap();
    assert!(text.contains("function calculateSummary(user, factor)"));
    assert!(text.contains("return { user, total };"));

    // Read signature view
    let sig_res = read_symbol(&app, "service.js", "calculateSummary", None, None, true);
    assert_eq!(sig_res["state"], "ok");
    assert_eq!(sig_res["data"]["view"], "signature");
    assert!(sig_res["data"]["text"].is_null());
    assert_eq!(sig_res["data"]["editReady"], false);
}

#[test]
fn test_ts_outline_and_read_symbol() {
    let root = tempfile::tempdir().unwrap();
    let ts_code = r#"export interface Config {
    port: number;
    host: string;
    connect(timeout: number): boolean;
}

export type Status = "active" | "inactive";

export enum Direction {
    Up,
    Down,
}

export class Server {
    start(): void {
        console.log("running");
    }
}
"#;

    std::fs::write(root.path().join("server.ts"), ts_code).unwrap();
    let app = adapters::application(root.path()).unwrap();

    let out = outline(&app, "server.ts", None);
    assert_eq!(out["state"], "ok");
    assert_eq!(out["data"]["language"], "typescript");

    let items = out["data"]["items"].as_array().unwrap();
    assert!(
        items
            .iter()
            .any(|s| s["name"] == "Config" && s["kind"] == "interface_declaration")
    );
    assert!(
        items
            .iter()
            .any(|s| s["name"] == "port" && s["container"] == "Config")
    );
    assert!(items.iter().any(|s| s["name"] == "connect"
        && s["container"] == "Config"
        && s["kind"] == "method_signature"));
    assert!(
        items
            .iter()
            .any(|s| s["name"] == "Status" && s["kind"] == "type_alias_declaration")
    );
    assert!(
        items
            .iter()
            .any(|s| s["name"] == "Direction" && s["kind"] == "enum_declaration")
    );
    assert!(
        items
            .iter()
            .any(|s| s["name"] == "Up" && s["container"] == "Direction")
    );
    assert!(
        items
            .iter()
            .any(|s| s["name"] == "Server" && s["kind"] == "class_declaration")
    );
    assert!(
        items
            .iter()
            .any(|s| s["name"] == "start" && s["container"] == "Server")
    );

    // Read symbol with container
    let read_start = read_symbol(&app, "server.ts", "start", Some("Server"), None, false);
    assert_eq!(read_start["state"], "ok");
    assert!(
        read_start["data"]["text"]
            .as_str()
            .unwrap()
            .contains("start(): void")
    );
}

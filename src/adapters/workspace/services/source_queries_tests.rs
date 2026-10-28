use super::*;
use crate::adapters::{Project, external_scopes::Registry, project::hash};
use harness_domain::contracts::{OutlineArgs, ReadSymbolArgs};
use std::{
    fs,
    sync::{Arc, Mutex},
};

#[test]
fn read_line_numbers_are_display_only_and_respect_the_selected_eof_line() {
    let dir = tempfile::tempdir().unwrap();
    let source = b"first\r\nsecond\r\n";
    fs::write(dir.path().join("example.txt"), source).unwrap();
    let service = SourceQueryService::new(
        Project::new(dir.path()).unwrap(),
        Arc::new(Mutex::new(Registry::default())),
    );
    let context = OperationContext::default();
    let read_args = |line_numbers| ReadArgs {
        external_scope: None,
        path: "example.txt".into(),
        start: 2,
        end: 3,
        compact: false,
        line_numbers,
        session: None,
    };

    let numbered = service.read(&read_args(true), &context).unwrap();
    assert_eq!(numbered["text"], "second\r\n");
    assert_eq!(numbered["numberedText"], "2 | second\r\n3 | ");
    assert_eq!(numbered["hash"], hash(source));

    let plain = service.read(&read_args(false), &context).unwrap();
    assert_eq!(plain["text"], numbered["text"]);
    assert_eq!(plain["hash"], numbered["hash"]);
    assert!(plain.get("numberedText").is_none());
}

#[test]
fn text_search_works_without_index_maintenance_state() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("example.txt"), "needle in source\n").unwrap();
    let service = SourceQueryService::new(
        Project::new(dir.path()).unwrap(),
        Arc::new(Mutex::new(Registry::default())),
    );

    let result = service
        .search(
            &SearchArgs {
                ast: None,
                regex: None,
                case_sensitive: None,
                external_scope: None,
                kind: harness_domain::contracts::Kind::Text,
                query: "needle".into(),
                min_lines: None,
                globs: None,
                scope: None,
                limit: None,
                context_lines: None,
                session: None,
            cursor: None,
            offset: None,
            only_matching: None,
            },
            &OperationContext::default(),
        )
        .unwrap();

    assert_eq!(result["matches"][0]["text"], "needle in source");
    assert!(!dir.path().join(".codegraph").exists());
}

#[test]
fn symbol_navigation_accepts_js_over_global_limit_and_rejects_over_symbol_budget() {
    let dir = tempfile::tempdir().unwrap();
    let mut large_js = String::from("function target() { return 42; }\n//");
    large_js.push_str(&"x".repeat(2 * 1024 * 1024));
    fs::write(dir.path().join("large.js"), large_js).unwrap();

    let service = SourceQueryService::new(
        Project::new(dir.path()).unwrap(),
        Arc::new(Mutex::new(Registry::default())),
    );
    let context = OperationContext::default();
    // The symbol budget must not silently raise the general source-read limit.
    let read_failure = service
        .read(
            &ReadArgs {
                external_scope: None,
                path: "large.js".into(),
                start: 1,
                end: 1,
                compact: false,
                line_numbers: false,
                session: None,
            },
            &context,
        )
        .unwrap_err();
    assert_eq!(read_failure.0, "SIZE");

    let outline = service
        .outline(
            &OutlineArgs {
                external_scope: None,
                path: "large.js".into(),
                query: None,
                max_chars: None,
                filter_kind: None,
                view: None,
                session: None,
            },
            &context,
        )
        .unwrap();
    assert!(outline["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["name"] == "target"));

    let symbol = service
        .read_symbol(
            &ReadSymbolArgs {
                external_scope: None,
                path: "large.js".into(),
                symbol: "target".into(),
                container: None,
                symbol_id: None,
                view: None,
                session: None,
            },
            &context,
        )
        .unwrap();
    assert!(symbol["text"].as_str().unwrap().contains("return 42"));

    let too_large = format!(
        "function target() {{ return 42; }}\n//{}",
        "x".repeat(16 * 1024 * 1024),
    );
    fs::write(dir.path().join("over-budget.js"), too_large).unwrap();
    let failure = service
        .outline(
            &OutlineArgs {
                external_scope: None,
                path: "over-budget.js".into(),
                query: None,
                max_chars: None,
                filter_kind: None,
                view: None,
                session: None,
            },
            &context,
        )
        .unwrap_err();
    assert_eq!(failure.0, "SIZE");
    assert!(failure.1.contains("16 MiB"));
}

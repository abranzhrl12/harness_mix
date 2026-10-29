use super::*;
use harness_domain::lsp::LspAction;
use std::time::{Duration, Instant};

fn exercise(extension: &str, language: &str, command: ServerCommand) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("tsconfig.json"), r#"{"compilerOptions":{"allowJs":true,"checkJs":true,"noEmit":true},"include":["*.ts","*.js"]}"#).unwrap();
    let target = format!("target.{extension}");
    let caller = format!("caller.{extension}");
    std::fs::write(dir.path().join(&target), "export function marker() {}\n").unwrap();
    let text = "import { marker } from './target';\nconst emoji = '😀'; marker();\n";
    std::fs::write(dir.path().join(&caller), text).unwrap();
    let client = Client::new(dir.path(), BTreeMap::from([(language.into(), command)]));
    let context = OperationContext::with_timeout(Duration::from_secs(120));
    let mut args = LspArgs {
        action: LspAction::Definition,
        path: caller.clone(),
        line: 1,
        character: 20,
        include_declaration: true,
        session: None,
    };
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let result = client.query(&args, text.into(), &context).unwrap();
        if result["locations"].as_array().unwrap().iter().any(|loc| {
            loc["path"] == target
                && loc["range"]["start"]["line"] == 0
                && loc["range"]["start"]["character"] == 16
        }) {
            break;
        }
        assert!(Instant::now() < deadline, "definition not ready: {result}");
        std::thread::sleep(Duration::from_millis(250));
    }
    args.action = LspAction::References;
    loop {
        let result = client.query(&args, text.into(), &context).unwrap();
        if result["locations"].as_array().unwrap().iter().any(|loc| {
            loc["path"] == caller
                && loc["range"]["start"]["line"] == 1
                && loc["range"]["start"]["character"] == 20
        }) {
            break;
        }
        assert!(Instant::now() < deadline, "references not ready: {result}");
        std::thread::sleep(Duration::from_millis(250));
    }
    // The same live server must observe new document contents.
    let changed = "import { marker } from './target';\n\nmarker();\n";
    std::fs::write(dir.path().join(&caller), changed).unwrap();
    args.line = 2;
    args.character = 1;
    args.action = LspAction::Definition;
    let result = client.query(&args, changed.into(), &context).unwrap();
    assert!(
        result["locations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|loc| loc["path"] == target)
    );
    // An editor/Harness can save another file before the next query. A cached
    // open document must not keep that old caller alive in references.
    std::fs::write(dir.path().join(&caller), "export {};\n").unwrap();
    args.path = target.clone();
    args.line = 0;
    args.character = 17;
    args.action = LspAction::References;
    let result = client
        .query(&args, "export function marker() {}\n".into(), &context)
        .unwrap();
    assert!(
        !result["locations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|loc| loc["path"] == caller && loc["range"]["start"]["line"] == 2),
        "stale caller: {result}"
    );
}

fn typescript_command() -> ServerCommand {
    ServerCommand {
        command: "node".into(),
        args: vec![
            std::env::var("HARNESS_TEST_TYPESCRIPT_LSP").expect(
                "HARNESS_TEST_TYPESCRIPT_LSP must point to typescript-language-server/lib/cli.mjs",
            ),
            "--stdio".into(),
        ],
    }
}
#[test]
#[ignore = "requires installed typescript-language-server and TypeScript; set HARNESS_TEST_TYPESCRIPT_LSP"]
fn typescript_definitions_references_and_changes() {
    exercise("ts", "typescript", typescript_command());
}
#[test]
#[ignore = "requires installed typescript-language-server and TypeScript; set HARNESS_TEST_TYPESCRIPT_LSP"]
fn javascript_definitions_references_and_changes() {
    exercise("js", "javascript", typescript_command());
}

#[test]
fn cancelled_query_never_launches_server() {
    let dir = tempfile::tempdir().unwrap();
    let client = Client::new(dir.path(), BTreeMap::new());
    let context = OperationContext::without_deadline();
    context.cancel();
    let args = LspArgs {
        action: LspAction::Definition,
        path: "file.ts".into(),
        line: 0,
        character: 0,
        include_declaration: false,
        session: None,
    };
    assert_eq!(
        client.query(&args, "".into(), &context).unwrap_err().0,
        "CANCELLED"
    );
}

//! Characterization of the published third-party client, not a Harness adapter.
use std::{fs, time::Duration};
use futures::FutureExt;
use lsp_types::{Position, Uri};
use serena_rs::lsp::{LspClient, path_from_uri, uri_from_path};
use serde_json::Value;

async fn probe(
    command: &str,
    args: &[String],
    manifest: (&str, &str),
    source_file: &str,
    source: &str,
    call_column: u32,
    declaration_column: u32,
) {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join(manifest.0), manifest.1).unwrap();
    let file = root.path().join(source_file);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, source).unwrap();
    let uri = uri_from_path(&file).unwrap();
    let mut client = tokio::time::timeout(
        Duration::from_secs(45),
        LspClient::start(command, args, root.path()),
    ).await.expect("server initialization exceeded deadline").expect("server failed to initialize");
    let outcome = std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(60), async {
        client.did_open(uri.clone(), if source_file.ends_with(".rs") { "rust" } else { "go" }, source.into()).await.unwrap();
        let position = Position::new(2, call_column);
        // Servers may initially report an unloaded document. Poll only the fixture,
        // with an outer deadline, and preserve the final successful response.
        let definition = loop {
            if let Ok(Some(result)) = client.definition(uri.clone(), position).await {
                let value = serde_json::to_value(result).unwrap();
                if !value.as_array().is_some_and(Vec::is_empty) {
                    break value;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        let locations: Vec<Value> = match definition {
            Value::Array(items) => items,
            item => vec![item],
        };
        assert!(locations.iter().any(|l| {
            let range = l.get("targetSelectionRange").or_else(|| l.get("range")).unwrap();
            let actual_uri = l.get("targetUri").or_else(|| l.get("uri"))
                .and_then(Value::as_str).and_then(|u| u.parse::<Uri>().ok());
            let actual_file = actual_uri.as_ref().and_then(path_from_uri)
                .and_then(|p| p.canonicalize().ok());
            actual_file == file.canonicalize().ok()
                && range["start"]["line"] == 1 && range["start"]["character"] == declaration_column
        }), "definition did not resolve the declaration: {locations:?}");
        let references = loop {
            if let Ok(Some(references)) = client.references(uri.clone(), position).await {
                if !references.is_empty() { break references; }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        assert!(references.iter().any(|r| {
            path_from_uri(&r.uri).and_then(|p| p.canonicalize().ok()) == file.canonicalize().ok()
                && r.range.start == position
        }),
            "call-site reference missing: {references:?}");
        serde_json::json!({
            "server": client.server_name(),
            "source": source_file,
            "definition": locations,
            "referenceCount": references.len(),
            "callSiteFound": true,
        })
    })).catch_unwind().await;
    let shutdown = tokio::time::timeout(Duration::from_secs(10), client.shutdown()).await;
    let result = match outcome {
        Ok(result) => result.expect("navigation exceeded deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    };
    assert!(shutdown.is_ok(), "server shutdown exceeded deadline");
    println!("{result}");
}

#[tokio::test]
async fn serena_rs_resolves_rust_definition_and_references() {
    probe(
        "rust-analyzer", &[],
        ("Cargo.toml", "[package]\nname='lsp_fixture'\nversion='0.1.0'\nedition='2021'\n[workspace]\n"),
        "src/lib.rs",
        "#![no_std]\npub fn add(a: u32, b: u32) -> u32 { a + b }\npub fn run() -> u32 { add(1, 2) }\n",
        22, 7,
    ).await;
}

#[tokio::test]
async fn serena_rs_resolves_go_definition_and_references() {
    probe(
        "gopls", &[],
        ("go.mod", "module fixture\n\ngo 1.24\n"),
        "fixture.go",
        "package fixture\nfunc Add(a, b int) int { return a+b }\nfunc Run() int { return Add(1,2) }\n",
        24, 5,
    ).await;
}

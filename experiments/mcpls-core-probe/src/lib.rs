#[cfg(test)]
mod tests {
    use futures::FutureExt;
    use mcpls_core::{
        config::LspServerConfig,
        lsp::{LspServer, ServerInitConfig},
    };
    use serde_json::{json, Value};
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{Duration, Instant},
    };
    use tempfile::TempDir;
    use tokio::time::timeout;
    use url::Url;

    const DEADLINE: Duration = Duration::from_secs(180);
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

    fn fixture(language: &str) -> (TempDir, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join(language);
        fs::create_dir_all(&root).unwrap();
        let (target, caller, target_text, caller_text) = match language {
            "rust" => {
                fs::write(root.join("Cargo.toml"), "[package]\nname = \"mcpls_fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[workspace]\n").unwrap();
                (
                    root.join("src/lib.rs"),
                    root.join("src/caller.rs"),
                    "pub fn marker() {}\npub mod caller;\n",
                    "pub fn run() { crate::marker(); }\n",
                )
            }
            "go" => {
                fs::write(root.join("go.mod"), "module probe.local/test\n\ngo 1.23\n").unwrap();
                (
                    root.join("target.go"),
                    root.join("main.go"),
                    "package main\n\nfunc marker() {}\n",
                    "package main\n\nfunc main() { marker() }\n",
                )
            }
            _ => unreachable!(),
        };
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&target, target_text).unwrap();
        fs::write(&caller, caller_text).unwrap();
        (temp, target, caller)
    }

    fn file_uri(path: &Path) -> String {
        Url::from_file_path(path.canonicalize().unwrap()).unwrap().to_string()
    }

    fn same_file(uri: &str, expected: &Path) -> bool {
        Url::parse(uri)
            .ok()
            .and_then(|u| u.to_file_path().ok())
            .and_then(|p| p.canonicalize().ok())
            .as_deref()
            == expected.canonicalize().ok().as_deref()
    }

    async fn probe(language: &str, command: &str, args: Vec<String>) {
        let (_temp, target, caller) = fixture(language);
        let mut config = match language {
            "rust" => LspServerConfig::rust_analyzer(),
            "go" => LspServerConfig::gopls(),
            _ => unreachable!(),
        };
        config.command = command.to_owned();
        config.args = args;
        let init = ServerInitConfig {
            server_config: config,
            workspace_roots: vec![if language == "rust" {
                caller.parent().unwrap().parent().unwrap().to_path_buf()
            } else {
                caller.parent().unwrap().to_path_buf()
            }],
            initialization_options: None,
            notification_tx: None,
        };
        let mut server = timeout(DEADLINE, LspServer::spawn(init))
            .await
            .expect("initialize deadline")
            .expect("LSP server initialization");
        let mut notifications = server.take_notification_rx();
        let notification_pump = tokio::spawn(async move {
            while notifications.recv().await.is_some() {}
        });
        let exercise = std::panic::AssertUnwindSafe(async {
            let capabilities = server.capabilities();
            assert!(capabilities.definition_provider.is_some(), "{language} lacks definition capability");
            assert!(capabilities.references_provider.is_some(), "{language} lacks references capability");

            let client = server.client();
            // 0.3.8 completes initialize/initialized in spawn but its client
            // state remains Initializing. It is not an indexing-readiness gate.
            // Validate known positive semantic responses below with a deadline.
            eprintln!("{language} state after handshake: {:?}", client.state().await);
            let caller_uri = file_uri(&caller);
            let target_uri = file_uri(&target);
            let content = fs::read_to_string(&caller).unwrap();
            client
                .notify(
                    "textDocument/didOpen",
                    json!({"textDocument":{"uri":caller_uri,"languageId":language,"version":1,"text":content}}),
                )
                .await
                .expect("didOpen");

            let position = if language == "rust" {
                json!({"line":0,"character":22})
            } else {
                json!({"line":2,"character":16})
            };
            let params = json!({"textDocument":{"uri":caller_uri},"position":position});
            let deadline = Instant::now() + DEADLINE;
            let locations = loop {
                let result: Value = timeout(REQUEST_TIMEOUT, client.request("textDocument/definition", params.clone(), REQUEST_TIMEOUT))
                    .await
                    .expect("definition request deadline")
                    .expect("definition request");
                let locations = as_locations(&result);
                if locations.iter().any(|loc| same_file(loc["uri"].as_str().unwrap_or_default(), &target)) {
                    break locations;
                }
                assert!(Instant::now() < deadline, "{language} definition not ready; last response: {result}");
                tokio::time::sleep(Duration::from_millis(500)).await;
            };
            let definition = locations.iter().find(|loc| same_file(loc["uri"].as_str().unwrap(), &target)).unwrap();
            let expected_start = if language == "rust" { (0, 7) } else { (2, 5) };
            assert_eq!(definition["range"]["start"]["line"], expected_start.0);
            assert_eq!(definition["range"]["start"]["character"], expected_start.1);

            let reference_params = json!({
                "textDocument":{"uri":target_uri},
                "position":{"line": if language == "rust" {0} else {2},"character": if language == "rust" {7} else {5}},
                "context":{"includeDeclaration":true}
            });
            let refs = loop {
                let result: Value = timeout(REQUEST_TIMEOUT, client.request("textDocument/references", reference_params.clone(), REQUEST_TIMEOUT))
                    .await
                    .expect("references request deadline")
                    .expect("references request");
                let locations = as_locations(&result);
                if locations.iter().any(|loc| same_file(loc["uri"].as_str().unwrap_or_default(), &target))
                    && locations.iter().any(|loc| same_file(loc["uri"].as_str().unwrap_or_default(), &caller))
                {
                    break locations;
                }
                assert!(Instant::now() < deadline, "{language} references not ready; last response: {result}");
                tokio::time::sleep(Duration::from_millis(500)).await;
            };
            assert!(refs.iter().any(|loc| same_file(loc["uri"].as_str().unwrap(), &caller)));
        }).catch_unwind().await;
        let shutdown_result = timeout(Duration::from_secs(15), server.shutdown()).await;
        notification_pump.abort();
        match exercise {
            Err(panic) => {
                let _ = shutdown_result;
                std::panic::resume_unwind(panic);
            }
            Ok(()) => shutdown_result.expect("shutdown deadline").expect("graceful shutdown"),
        }
    }

    fn as_locations(value: &Value) -> Vec<Value> {
        let entries = if let Some(list) = value.as_array() {
            list.clone()
        } else if value.is_null() {
            Vec::new()
        } else {
            vec![value.clone()]
        };
        entries.into_iter().map(|entry| {
            if let Some(uri) = entry.get("targetUri") {
                json!({"uri":uri,"range":entry.get("targetSelectionRange").or_else(|| entry.get("targetRange"))})
            } else {
                entry
            }
        }).collect()
    }

    #[tokio::test]
    async fn rust_analyzer_definition_and_references() {
        probe("rust", "rust-analyzer", Vec::new()).await;
    }

    #[tokio::test]
    async fn gopls_definition_and_references() {
        probe("go", "gopls", Vec::new()).await;
    }
}

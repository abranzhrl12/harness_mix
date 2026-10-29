use super::*;
use harness_domain::lsp::LspAction;
use mcpls_core::{
    config::LspServerConfig,
    lsp::{LspServer, ServerInitConfig},
};
use serde_json::json;
use url::Url;

pub(super) fn run(root: PathBuf, commands: Commands, mut rx: tokio::sync::mpsc::Receiver<Query>) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    runtime.block_on(async {
        let mut servers = BTreeMap::<String, LiveServer>::new();
        while let Some(query) = rx.recv().await {
            let result = run_query(&root, &commands, &mut servers, &query).await;
            let _ = query.reply.send(result);
        }
        for (_, server) in servers {
            server.close().await;
        }
    });
}
struct LiveServer {
    server: LspServer,
    pump: tokio::task::JoinHandle<()>,
    documents: BTreeMap<String, (String, i32)>,
}
impl LiveServer {
    async fn close(self) {
        // 0.8 bounds its handshake and process-tree sweep internally. Do not
        // cancel that teardown midway with a shorter outer timeout.
        let _ = self.server.shutdown().await;
        self.pump.abort();
    }
}
async fn guarded<T>(
    context: &OperationContext,
    future: impl std::future::Future<Output = Result<T, Failure>>,
) -> Result<T, Failure> {
    tokio::pin!(future);
    loop {
        context.check()?;
        tokio::select! {
            result = &mut future => return result,
            _ = tokio::time::sleep(Duration::from_millis(25)) => {}
        }
    }
}
async fn run_query(
    root: &Path,
    commands: &Commands,
    servers: &mut BTreeMap<String, LiveServer>,
    query: &Query,
) -> Result<Value, Failure> {
    query.context.check()?;
    let language = language(&query.args.path)?;
    if !servers.contains_key(language) {
        let command = &commands[language];
        let mut config = match language {
            "go" => LspServerConfig::gopls(),
            "typescript" | "javascript" => LspServerConfig::typescript(),
            _ => LspServerConfig::rust_analyzer(),
        };
        config.command = mcpls_core::config::LaunchCommand::explicit(
            mcpls_core::config::ServerCommand::new(command.command.clone())
                .map_err(|_| ("LSP_CONFIG", "Invalid operator LSP command"))?,
        );
        config.args = serde_json::from_value(json!(command.args))
            .map_err(|_| ("LSP_CONFIG", "Invalid operator LSP arguments"))?;
        config.language_id = serde_json::from_value(json!(language))
            .map_err(|_| ("LSP_CONFIG", "Invalid LSP language"))?;
        let configured_root = mcpls_core::config::ConfiguredRoot::new(root)
            .map_err(|_| ("PATH", "Invalid LSP workspace root"))?;
        let roots = mcpls_core::bridge::WorkspaceRoots::from_configured(&[configured_root])
            .map_err(|_| ("PATH", "LSP workspace root unavailable"))?;
        let init = ServerInitConfig::new(
            config,
            roots,
            mcpls_core::config::PositionEncodings::DEFAULT,
            std::sync::Arc::default(),
        );
        let context = query
            .context
            .with_execution_timeout(Some(Duration::from_secs(30)));
        let spawned = guarded(&context, async { LspServer::spawn(init).await.map_err(|_| ("LSP_START", "Cannot initialize configured LSP server; verify installation and operator configuration")) }).await?;
        let server = spawned.server;
        let mut lanes = spawned.lanes;
        let pump = tokio::spawn(async move {
            tokio::join!(
                async { while lanes.notifications.messages.recv().await.is_some() {} },
                async { while lanes.notifications.publishes.recv().await.is_some() {} },
                async { while lanes.lifecycle.recv().await.is_some() {} },
            );
        });
        servers.insert(
            language.into(),
            LiveServer {
                server,
                pump,
                documents: BTreeMap::new(),
            },
        );
    }
    let live = servers.get_mut(language).unwrap();
    let capabilities = live.server.capabilities();
    let supported = match query.args.action {
        LspAction::Definition => capabilities
            .definition_provider
            .as_ref()
            .is_some_and(|v| serde_json::to_value(v).is_ok_and(|v| v != false)),
        LspAction::References => capabilities
            .references_provider
            .as_ref()
            .is_some_and(|v| serde_json::to_value(v).is_ok_and(|v| v != false)),
    };
    if !supported {
        return Err((
            "UNSUPPORTED",
            "Configured LSP server does not advertise this operation",
        ));
    }
    let uri = Url::from_file_path(
        root.join(&query.args.path)
            .canonicalize()
            .map_err(|_| ("PATH", "LSP source unavailable"))?,
    )
    .map_err(|_| ("PATH", "Cannot form LSP document URI"))?
    .to_string();
    let encoding = live.server.position_encoding();
    let encoding = encoding.as_str().to_owned();
    let character = encoded_character(
        &query.text,
        query.args.line,
        query.args.character,
        &encoding,
    )?;
    let document_language = document_language(&query.args.path)?;
    let client = live.server.client();
    let context = query
        .context
        .with_execution_timeout(Some(Duration::from_secs(20)));
    let result = guarded(&context, async {
        // Only the active query document stays open. Saved changes to other
        // files must be read from disk rather than a stale editor overlay.
        let previous = live.documents.keys().filter(|open| *open != &uri).cloned().collect::<Vec<_>>();
        for previous in previous {
            client.notify("textDocument/didClose", json!({"textDocument":{"uri":previous}})).await
                .map_err(|_| ("LSP_SERVER", "LSP document close failed"))?;
            live.documents.remove(&previous);
        }
        match live.documents.get(&uri) {
            None => {
                client.notify("textDocument/didOpen", json!({"textDocument":{"uri":uri,"languageId":document_language,"version":1,"text":query.text}})).await.map_err(|_| ("LSP_SERVER", "LSP document open failed"))?;
                live.documents.insert(uri.clone(), (query.text.clone(), 1));
            }
            Some((previous, version)) if previous != &query.text => {
                let version = version.checked_add(1).ok_or(("LSP_SERVER", "LSP document version exhausted"))?;
                client.notify("textDocument/didChange", json!({"textDocument":{"uri":uri,"version":version},"contentChanges":[{"text":query.text}]})).await.map_err(|_| ("LSP_SERVER", "LSP document synchronization failed"))?;
                live.documents.insert(uri.clone(), (query.text.clone(), version));
            }
            _ => {}
        }
        let method = match query.args.action { LspAction::Definition => "textDocument/definition", LspAction::References => "textDocument/references" };
        let mut params = json!({"textDocument":{"uri":uri},"position":{"line":query.args.line,"character":character}});
        if matches!(query.args.action, LspAction::References) { params["context"] = json!({"includeDeclaration":query.args.include_declaration}); }
        let response: Value = client.request(method, params, Duration::from_secs(20)).await.map_err(|error| {
            #[cfg(test)]
            eprintln!("LSP request diagnostic: {error:?}");
            match error {
                mcpls_core::error::Error::ServerTerminated => ("LSP_SERVER_STOPPED", "LSP server connection stopped; the session was discarded and the next query starts a new server"),
                mcpls_core::error::Error::LspProtocolError(_) => ("LSP_PROTOCOL", "LSP protocol response was invalid; the session was discarded; inspect the configured server"),
                _ => ("LSP_QUERY", "LSP request failed; the session was discarded; inspect the configured server and retry"),
            }
        })?;
        Ok(response)
    }).await;
    let response = match result {
        Ok(value) => value,
        Err(error) => {
            if let Some(server) = servers.remove(language) {
                server.close().await;
            }
            return Err(error);
        }
    };
    let (locations, omitted, truncated) = locations(root, response)?;
    Ok(
        json!({"language":language,"action":query.args.action,"path":query.args.path,"sourceHash":harness_domain::hash(query.text.as_bytes()),
        "inputPositionEncoding":"utf-16","positionEncoding":encoding,"locations":locations,"truncated":truncated,"omittedOutsideWorkspace":omitted,
        "coverage":"server-response-only","indexReadiness":"unknown","absenceProven":false,"documentSync":"full",
        "serverStateNotice":"A completed LSP handshake does not guarantee workspace indexing is complete"}),
    )
}
pub(super) fn document_language(path: &str) -> Result<&'static str, Failure> {
    match Path::new(path).extension().and_then(|s| s.to_str()) {
        Some("tsx") => Ok("typescriptreact"),
        Some("jsx") => Ok("javascriptreact"),
        _ => language(path),
    }
}
pub(super) fn encoded_character(
    text: &str,
    line: u32,
    character: u32,
    encoding: &str,
) -> Result<u32, Failure> {
    if !matches!(encoding, "utf-8" | "utf-16" | "utf-32") {
        return Err(("UNSUPPORTED", "Unknown LSP position encoding"));
    }
    let line = text
        .split('\n')
        .nth(line as usize)
        .ok_or(("ARGUMENT", "LSP line outside source"))?;
    let mut utf16 = 0;
    let mut units = 0;
    for ch in line.chars() {
        if utf16 == character {
            return Ok(units);
        }
        utf16 += ch.len_utf16() as u32;
        units += match encoding {
            "utf-8" => ch.len_utf8() as u32,
            "utf-16" => ch.len_utf16() as u32,
            "utf-32" => 1,
            _ => return Err(("UNSUPPORTED", "Unknown LSP position encoding")),
        };
    }
    if utf16 == character {
        Ok(units)
    } else {
        Err(("ARGUMENT", "Invalid LSP position"))
    }
}
fn locations(root: &Path, response: Value) -> Result<(Vec<Value>, usize, bool), Failure> {
    let entries = match response {
        Value::Null => vec![],
        Value::Array(v) => v,
        Value::Object(_) => vec![response],
        _ => return Err(("LSP_RESPONSE", "Malformed LSP locations response")),
    };
    let mut result = Vec::new();
    let mut outside = 0;
    let mut truncated = false;
    for entry in entries {
        let uri = entry
            .get("targetUri")
            .or_else(|| entry.get("uri"))
            .and_then(Value::as_str)
            .ok_or(("LSP_RESPONSE", "LSP location has no URI"))?;
        let range = entry
            .get("targetSelectionRange")
            .or_else(|| entry.get("range"))
            .ok_or(("LSP_RESPONSE", "LSP location has no range"))?;
        let path = Url::parse(uri)
            .ok()
            .and_then(|uri| uri.to_file_path().ok())
            .and_then(|path| path.canonicalize().ok());
        let Some(path) = path.filter(|path| path.starts_with(root)) else {
            outside += 1;
            continue;
        };
        if result.len() == 100 {
            truncated = true;
            continue;
        }
        result.push(json!({"path":path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"),"range":range}));
    }
    Ok((result, outside, truncated))
}

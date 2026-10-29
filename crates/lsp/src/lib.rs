//! Bounded read-only LSP adapter. Commands come from the operator, never tool
//! arguments or repository configuration. A worker owns servers and their runtime.
use harness_domain::{Failure, execution::OperationContext, lsp::LspArgs};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Mutex, mpsc},
    time::Duration,
};
mod worker;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServerCommand {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}
pub type Commands = BTreeMap<String, ServerCommand>;
pub fn default_commands() -> Commands {
    BTreeMap::from([
        (
            "rust".into(),
            ServerCommand {
                command: "rust-analyzer".into(),
                args: vec![],
            },
        ),
        (
            "go".into(),
            ServerCommand {
                command: "gopls".into(),
                args: vec![],
            },
        ),
        (
            "typescript".into(),
            ServerCommand {
                command: "typescript-language-server".into(),
                args: vec!["--stdio".into()],
            },
        ),
        (
            "javascript".into(),
            ServerCommand {
                command: "typescript-language-server".into(),
                args: vec!["--stdio".into()],
            },
        ),
    ])
}
pub fn language(path: &str) -> Result<&'static str, Failure> {
    match Path::new(path).extension().and_then(|s| s.to_str()) {
        Some("rs") => Ok("rust"),
        Some("go") => Ok("go"),
        Some("ts" | "tsx" | "mts" | "cts") => Ok("typescript"),
        Some("js" | "jsx" | "mjs" | "cjs") => Ok("javascript"),
        _ => Err((
            "UNSUPPORTED",
            "LSP supports Rust, Go, TypeScript and JavaScript source extensions",
        )),
    }
}
struct Query {
    args: LspArgs,
    text: String,
    context: OperationContext,
    reply: mpsc::SyncSender<Result<Value, Failure>>,
}
pub struct Client {
    root: PathBuf,
    commands: Commands,
    sender: Mutex<Option<tokio::sync::mpsc::Sender<Query>>>,
}
impl Client {
    pub fn new(root: &Path, commands: Commands) -> Self {
        Self {
            root: root.to_owned(),
            commands,
            sender: Mutex::new(None),
        }
    }
    pub fn query(
        &self,
        args: &LspArgs,
        text: String,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        context.check()?;
        let language = language(&args.path)?;
        if !self.commands.contains_key(language) {
            return Err((
                "UNSUPPORTED",
                "No operator-configured LSP server for this language",
            ));
        }
        if text.len() > 2 * 1024 * 1024 {
            return Err(("SIZE", "LSP document exceeds 2 MiB"));
        }
        harness_domain::lsp::validate_position(&text, args.line, args.character)?;
        let root = self
            .root
            .canonicalize()
            .map_err(|_| ("PATH", "LSP workspace unavailable"))?;
        let path = root
            .join(&args.path)
            .canonicalize()
            .map_err(|_| ("PATH", "LSP source unavailable"))?;
        if !path.starts_with(&root) || !path.is_file() {
            return Err(("PATH", "LSP source must be inside the workspace"));
        }
        let sender = {
            let mut sender = self
                .sender
                .lock()
                .map_err(|_| ("INTERNAL", "LSP worker lock unavailable"))?;
            if sender.is_none() {
                let (tx, rx) = tokio::sync::mpsc::channel::<Query>(4);
                let commands = self.commands.clone();
                std::thread::Builder::new()
                    .name("harness-lsp".into())
                    .spawn(move || worker::run(root, commands, rx))
                    .map_err(|_| ("INTERNAL", "LSP worker could not start"))?;
                *sender = Some(tx);
            }
            sender.as_ref().unwrap().clone()
        };
        let (reply, result) = mpsc::sync_channel(1);
        sender
            .try_send(Query {
                args: args.clone(),
                text,
                context: context.clone(),
                reply,
            })
            .map_err(|error| match error {
                tokio::sync::mpsc::error::TrySendError::Full(_) => {
                    ("BUSY", "LSP query queue is full")
                }
                tokio::sync::mpsc::error::TrySendError::Closed(_) => {
                    ("LSP_SERVER", "LSP worker stopped")
                }
            })?;
        loop {
            context.check()?;
            match result.recv_timeout(Duration::from_millis(25)) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(("LSP_SERVER", "LSP worker stopped"));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}
#[cfg(test)]
mod position_tests;
#[cfg(test)]
mod tests;

//! ACP v1 stdio adapter for external coding agents.
//!
//! The adapter owns the child process and speaks newline-delimited JSON-RPC.
//! It deliberately accepts an executable and MCP server descriptors from its
//! caller: an installed CLI is not assumed to also be an ACP agent.

use harness_application::ports::conversation::{ConversationFuture, ConversationProvider};
use base64::Engine as _;
use harness_domain::conversation::{
    AuthMethod, ConfigValue, ConnectionInfo, ConversationError, ConversationEvent,
    ConversationObserver, ImageAttachment, PermissionDecision, PermissionOption,
    PermissionRequest, SessionConfigOption, SessionInfo, SessionUsage, UsageCost,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use std::{
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{Mutex, mpsc},
};

const ACP_PROTOCOL_VERSION: i64 = 1;
const MAX_PROTOCOL_LINE_BYTES: usize = 8 * 1024 * 1024;
const MAX_STDERR_LINE_BYTES: usize = 16 * 1024;
const MAX_AUTH_URL_BYTES: usize = 8 * 1024;

/// Converts Rust's Windows verbatim paths to the conventional absolute form
/// accepted by Antigravity's Go ACP server. All other platform paths are kept.
pub fn protocol_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        use std::{
            os::windows::ffi::{OsStrExt, OsStringExt},
            path::{Component, Prefix},
        };

        let Some(Component::Prefix(prefix)) = path.components().next() else {
            return path.to_path_buf();
        };
        let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        match prefix.kind() {
            Prefix::VerbatimDisk(_) if wide.len() >= 4 => {
                PathBuf::from(OsString::from_wide(&wide[4..]))
            }
            Prefix::VerbatimUNC(_, _) if wide.len() >= 8 => {
                let mut normalized = vec![b'\\' as u16, b'\\' as u16];
                normalized.extend_from_slice(&wide[8..]);
                PathBuf::from(OsString::from_wide(&normalized))
            }
            _ => path.to_path_buf(),
        }
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

/// Process and session configuration supplied by the composition root.
#[derive(Clone, Debug)]
pub struct AntigravityAcpConfig {
    /// Absolute path to the ACP server executable, not `agy.exe`.
    pub executable: PathBuf,
    pub args: Vec<OsString>,
    /// ACP `session/new.params.mcpServers` entries (including Harness).
    pub mcp_servers: Vec<Value>,
    /// Optional path to the official local Harness bridge/helper.
    pub harness_helper: Option<PathBuf>,
}

impl AntigravityAcpConfig {
    pub fn new(executable: PathBuf) -> Self {
        Self {
            executable,
            args: Vec::new(),
            mcp_servers: Vec::new(),
            harness_helper: None,
        }
    }
}

#[derive(Clone)]
pub struct AntigravityAcpAdapter {
    config: Arc<AntigravityAcpConfig>,
    state: Arc<Mutex<Option<AcpProcess>>>,
    cancel_stdin: Arc<Mutex<Option<Arc<Mutex<ChildStdin>>>>>,
    next_id: Arc<Mutex<u64>>,
}

struct AcpProcess {
    child: Child,
    #[cfg(windows)]
    _process_tree: crate::adapters::process_tree::Tree,
    stdin: Arc<Mutex<ChildStdin>>,
    messages: mpsc::Receiver<Result<Value, String>>,
    auth_urls: mpsc::Receiver<String>,
    agent_name: String,
    auth_methods: Vec<AuthDescriptor>,
    initialized: bool,
    image_supported: bool,
    session_config_options: HashMap<String, Vec<SessionConfigOption>>,
}

#[derive(Clone, Debug)]
struct AuthDescriptor {
    method: AuthMethod,
    kind: String,
}

impl AntigravityAcpAdapter {
    pub fn new(config: AntigravityAcpConfig) -> Self {
        Self {
            config: Arc::new(config),
            state: Arc::new(Mutex::new(None)),
            cancel_stdin: Arc::new(Mutex::new(None)),
            next_id: Arc::new(Mutex::new(1)),
        }
    }

    async fn spawn_process(&self) -> Result<AcpProcess, ConversationError> {
        if !self.config.executable.is_absolute() {
            return Err(other("ACP executable path must be absolute"));
        }
        let mut command = Command::new(&self.config.executable);
        command.kill_on_drop(true);
        command.args(&self.config.args);
        if let Some(helper) = &self.config.harness_helper {
            if !helper.is_absolute() {
                return Err(other("Harness helper path must be absolute"));
            }
            command.env("ANTIGRAVITY_HARNESS_PATH", helper);
        }
        #[cfg(windows)]
        crate::adapters::process_tree::configure(command.as_std_mut());
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|e| other(format!("could not start ACP agent: {e}")))?;
        #[cfg(windows)]
        let process_tree = {
            let raw_handle = child
                .raw_handle()
                .ok_or_else(|| other("ACP agent exited before process-tree attachment"))?;
            let process_id = child
                .id()
                .ok_or_else(|| other("ACP agent process ID is unavailable"))?;
            match crate::adapters::process_tree::Tree::attach_handle(raw_handle, process_id) {
                Ok(tree) => tree,
                Err(error) => {
                    let _ = child.kill().await;
                    return Err(other(format!(
                        "could not contain ACP process tree: {error}"
                    )));
                }
            }
        };
        let stdin = Arc::new(Mutex::new(
            child
                .stdin
                .take()
                .ok_or_else(|| other("ACP agent stdin was not piped"))?,
        ));
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| other("ACP agent stdout was not piped"))?;
        let (message_tx, messages) = mpsc::channel(64);
        tokio::spawn(async move {
            let mut stdout = BufReader::new(stdout);
            let mut line = Vec::with_capacity(4096);
            loop {
                let message = read_protocol_message(&mut stdout, &mut line)
                    .await
                    .map_err(|error| error.to_string());
                let done = message.is_err();
                if message_tx.send(message).await.is_err() || done {
                    break;
                }
            }
        });
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| other("ACP agent stderr was not piped"))?;
        let (auth_url_tx, auth_urls) = mpsc::channel(8);
        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            let mut line = Vec::with_capacity(512);
            let mut overflow = false;
            loop {
                let count = match stderr.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(count) => count,
                };
                for byte in &buf[..count] {
                    if *byte == b'\n' {
                        if !overflow {
                            if let Ok(line_text) = std::str::from_utf8(&line) {
                                if let Some(url) = google_auth_url(line_text) {
                                    let _ = auth_url_tx.try_send(url);
                                }
                            }
                        }
                        line.clear();
                        overflow = false;
                    } else if !overflow {
                        if line.len() < MAX_STDERR_LINE_BYTES {
                            line.push(*byte);
                        } else {
                            line.clear();
                            overflow = true;
                        }
                    }
                }
            }
        });
        Ok(AcpProcess {
            child,
            #[cfg(windows)]
            _process_tree: process_tree,
            stdin,
            messages,
            auth_urls,
            agent_name: "Antigravity".to_owned(),
            auth_methods: Vec::new(),
            initialized: false,
            image_supported: false,
            session_config_options: HashMap::new(),
        })
    }

    async fn id(&self) -> u64 {
        let mut id = self.next_id.lock().await;
        let current = *id;
        *id = id.saturating_add(1);
        current
    }

    async fn ensure_started(
        &self,
        state: &mut Option<AcpProcess>,
        observer: &dyn ConversationObserver,
    ) -> Result<ConnectionInfo, ConversationError> {
        if state.is_none() {
            let started = Instant::now();
            let process = self.spawn_process().await?;
            *self.cancel_stdin.lock().await = Some(process.stdin.clone());
            *state = Some(process);
            report_acp_timing(observer, "spawn", started.elapsed());
        }
        let process = state.as_mut().expect("process was started");
        if !process.initialized {
            let id = self.id().await;
            let result = request(
                process,
                id,
                "initialize",
                json!({
                    "protocolVersion": ACP_PROTOCOL_VERSION,
                    "clientInfo": {"name": "harnessmix", "version": env!("CARGO_PKG_VERSION")},
                    "clientCapabilities": {
                        "auth": {"terminal": false},
                        "fs": {"readTextFile": false, "writeTextFile": false},
                        "terminal": false
                    }
                }),
                observer,
            )
            .await?;
            let version = result.get("protocolVersion").and_then(Value::as_i64);
            if version != Some(ACP_PROTOCOL_VERSION) {
                return Err(other(format!(
                    "ACP protocol version is unsupported: {version:?}"
                )));
            }
            process.agent_name = result
                .pointer("/agentInfo/title")
                .or_else(|| result.pointer("/agentInfo/name"))
                .and_then(Value::as_str)
                .unwrap_or("Antigravity")
                .to_owned();
            process.auth_methods = parse_auth_methods(result.get("authMethods"));
            process.image_supported = result.pointer("/agentCapabilities/promptCapabilities/image")
                .and_then(Value::as_bool).unwrap_or(false);
            process.initialized = true;
            notify(
                observer,
                ConversationEvent::Status(format!("Conectado a {} por ACP", process.agent_name)),
            )?;
        }
        Ok(connection_info(process))
    }

    async fn roundtrip(
        &self,
        process: &mut AcpProcess,
        method: &str,
        params: Value,
        observer: &dyn ConversationObserver,
    ) -> Result<Value, ConversationError> {
        let id = self.id().await;
        request(process, id, method, params, observer).await
    }
}

impl ConversationProvider for AntigravityAcpAdapter {
    fn initialize<'a>(
        &'a self,
        observer: &'a dyn ConversationObserver,
    ) -> ConversationFuture<'a, ConnectionInfo> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            self.ensure_started(&mut state, observer).await
        })
    }

    fn authenticate<'a>(
        &'a self,
        method_id: &'a str,
        observer: &'a dyn ConversationObserver,
    ) -> ConversationFuture<'a, ConnectionInfo> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            self.ensure_started(&mut state, observer).await?;
            let process = state.as_mut().expect("initialized process");
            let descriptor = process
                .auth_methods
                .iter()
                .find(|candidate| candidate.method.id == method_id)
                .cloned()
                .ok_or_else(|| {
                    other("authentication method was not advertised by the ACP agent")
                })?;
            if descriptor.kind == "terminal" {
                return Err(other(
                    "this advertised login method requires a separate interactive terminal; protocol authenticate must not be called for it",
                ));
            }
            self.roundtrip(
                process,
                "authenticate",
                json!({"methodId": descriptor.method.id}),
                observer,
            )
            .await?;
            Ok(connection_info(process))
        })
    }

    fn new_session<'a>(
        &'a self,
        workspace: &'a Path,
        observer: &'a dyn ConversationObserver,
    ) -> ConversationFuture<'a, SessionInfo> {
        Box::pin(async move {
            let canonical_cwd = workspace
                .canonicalize()
                .map_err(|e| other(format!("cannot resolve workspace path: {e}")))?;
            let cwd = protocol_path(&canonical_cwd);
            let mut state = self.state.lock().await;
            self.ensure_started(&mut state, observer).await?;
            let process = state.as_mut().expect("initialized process");
            let result = self
                .roundtrip(
                    process,
                    "session/new",
                    json!({"cwd": cwd, "mcpServers": self.config.mcp_servers}),
                    observer,
                )
                .await?;
            let id = result
                .get("sessionId")
                .and_then(Value::as_str)
                .ok_or_else(|| other("ACP session/new response did not contain sessionId"))?;
            let config_options = parse_config_options(result.get("configOptions"));
            process
                .session_config_options
                .insert(id.to_owned(), config_options.clone());
            Ok(SessionInfo {
                id: id.to_owned(),
                config_options,
            })
        })
    }

    fn set_config_option<'a>(
        &'a self,
        session_id: &'a str,
        config_id: &'a str,
        value: &'a str,
        observer: &'a dyn ConversationObserver,
    ) -> ConversationFuture<'a, Vec<SessionConfigOption>> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            let process = state
                .as_mut()
                .ok_or_else(|| other("ACP agent is not initialized"))?;
            let advertised = process
                .session_config_options
                .get(session_id)
                .is_some_and(|options| advertises_config_value(options, config_id, value));
            if !advertised {
                return Err(other(
                    "ACP configuration value was not advertised for this session",
                ));
            }
            let result = self
                .roundtrip(
                    process,
                    "session/set_config_option",
                    json!({"sessionId":session_id,"configId":config_id,"value":value}),
                    observer,
                )
                .await?;
            if !result
                .get("configOptions")
                .is_some_and(Value::is_array)
            {
                return Err(other(
                    "ACP set_config_option response omitted configOptions",
                ));
            }
            let config_options = parse_config_options(result.get("configOptions"));
            process
                .session_config_options
                .insert(session_id.to_owned(), config_options.clone());
            notify(
                observer,
                ConversationEvent::ConfigOptions(config_options.clone()),
            )?;
            Ok(config_options)
        })
    }

    fn prompt<'a>(
        &'a self,
        session_id: &'a str,
        text: &'a str,
        observer: &'a dyn ConversationObserver,
    ) -> ConversationFuture<'a, String> {
        self.prompt_with_images(session_id, text, &[], observer)
    }

    fn prompt_with_images<'a>(
        &'a self,
        session_id: &'a str,
        text: &'a str,
        images: &'a [ImageAttachment],
        observer: &'a dyn ConversationObserver,
    ) -> ConversationFuture<'a, String> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            let process = state
                .as_mut()
                .ok_or_else(|| other("ACP agent is not initialized"))?;
            if !images.is_empty() && !process.image_supported {
                return Err(other("El agente no anunció soporte de imágenes mediante ACP"));
            }
            let prompt = encode_prompt(text, images)?;
            let result = self
                .roundtrip(
                    process,
                    "session/prompt",
                    json!({
                        "sessionId": session_id,
                        "prompt": prompt
                    }),
                    observer,
                )
                .await?;
            Ok(result
                .get("stopReason")
                .and_then(Value::as_str)
                .unwrap_or("end_turn")
                .to_owned())
        })
    }

    fn cancel<'a>(&'a self, session_id: &'a str) -> ConversationFuture<'a, ()> {
        Box::pin(async move {
            let stdin = self
                .cancel_stdin
                .lock()
                .await
                .clone()
                .ok_or_else(|| other("ACP agent is not initialized"))?;
            let mut bytes = serde_json::to_vec(&json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":session_id}}))
                .map_err(|e| other(format!("could not encode ACP cancellation: {e}")))?;
            bytes.push(b'\n');
            let mut stdin = stdin.lock().await;
            stdin
                .write_all(&bytes)
                .await
                .map_err(|e| other(format!("ACP cancellation write failed: {e}")))?;
            stdin
                .flush()
                .await
                .map_err(|e| other(format!("ACP cancellation flush failed: {e}")))
        })
    }

    fn close<'a>(&'a self) -> ConversationFuture<'a, ()> {
        Box::pin(async move {
            let mut state = self.state.lock().await;
            *self.cancel_stdin.lock().await = None;
            if let Some(mut process) = state.take() {
                let _ = process.child.kill().await;
                let _ = process.child.wait().await;
            }
            Ok(())
        })
    }
}

fn encode_prompt(text: &str, images: &[ImageAttachment]) -> Result<Vec<Value>, ConversationError> {
    let total = images.iter().try_fold(0usize, |total, image| total.checked_add(image.data.len()))
        .ok_or_else(|| other("Adjuntos demasiado grandes"))?;
    if total > 20 * 1024 * 1024 {
        return Err(other("Los adjuntos de un mensaje no pueden superar 20 MiB"));
    }
    let mut content = Vec::with_capacity(images.len() + 1);
    if !text.is_empty() {
        content.push(json!({"type":"text", "text":text}));
    }
    for image in images {
        if !matches!(image.mime_type.as_str(), "image/png" | "image/jpeg" | "image/gif" | "image/webp") || image.data.is_empty() {
            return Err(other("Formato de imagen no soportado o imagen vacía"));
        }
        content.push(json!({"type":"image", "mimeType":image.mime_type,
            "data":base64::engine::general_purpose::STANDARD.encode(&image.data)}));
    }
    if content.is_empty() { return Err(other("El mensaje está vacío")); }
    Ok(content)
}

fn connection_info(process: &AcpProcess) -> ConnectionInfo {
    ConnectionInfo {
        agent_name: process.agent_name.clone(),
        auth_methods: process
            .auth_methods
            .iter()
            .map(|auth| auth.method.clone())
            .collect(),
    }
}

fn parse_auth_methods(methods: Option<&Value>) -> Vec<AuthDescriptor> {
    methods
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|method| {
            let id = method.get("id")?.as_str()?.to_owned();
            Some(AuthDescriptor {
                method: AuthMethod {
                    id,
                    name: method
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("Sign in")
                        .to_owned(),
                    description: {
                        let description = method
                            .get("description")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        if method.get("type").and_then(Value::as_str) == Some("terminal") {
                            Some(match description {
                                Some(description) => {
                                    format!("{description} (requires interactive terminal)")
                                }
                                None => "Requires interactive terminal".to_owned(),
                            })
                        } else {
                            description
                        }
                    },
                },
                kind: method
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("agent")
                    .to_owned(),
            })
        })
        .collect()
}

fn parse_config_value(value: &Value) -> Option<ConfigValue> {
    Some(ConfigValue {
        value: value.get("value")?.as_str()?.to_owned(),
        name: value.get("name")?.as_str()?.to_owned(),
    })
}

fn parse_config_options(value: Option<&Value>) -> Vec<SessionConfigOption> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|option| {
            // The client advertises only ACP's baseline `select` config type.
            if option.get("type").and_then(Value::as_str) != Some("select") {
                return None;
            }
            let values = option.get("options")?.as_array()?;
            let mut parsed_values = Vec::new();
            for entry in values {
                if let Some(grouped) = entry.get("options").and_then(Value::as_array) {
                    parsed_values.extend(grouped.iter().filter_map(parse_config_value));
                } else if let Some(value) = parse_config_value(entry) {
                    parsed_values.push(value);
                }
            }
            Some(SessionConfigOption {
                id: option.get("id")?.as_str()?.to_owned(),
                name: option.get("name")?.as_str()?.to_owned(),
                category: option
                    .get("category")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                current_value: option.get("currentValue")?.as_str()?.to_owned(),
                options: parsed_values,
            })
        })
        .collect()
}

fn advertises_config_value(
    options: &[SessionConfigOption],
    config_id: &str,
    value: &str,
) -> bool {
    options
        .iter()
        .find(|option| option.id == config_id)
        .is_some_and(|option| option.options.iter().any(|candidate| candidate.value == value))
}

fn parse_session_usage(update: &Value) -> Option<SessionUsage> {
    let used = update.get("used")?.as_u64()?;
    let size = update.get("size")?.as_u64()?;
    if size == 0 {
        return None;
    }
    let cost = update.get("cost").and_then(|cost| {
        let amount = cost.get("amount")?.as_f64()?;
        let currency = cost.get("currency")?.as_str()?;
        if !amount.is_finite() || currency.is_empty() {
            return None;
        }
        Some(UsageCost {
            amount,
            currency: currency.to_owned(),
        })
    });
    Some(SessionUsage { used, size, cost })
}

async fn request(
    process: &mut AcpProcess,
    id: u64,
    method: &str,
    params: Value,
    observer: &dyn ConversationObserver,
) -> Result<Value, ConversationError> {
    let started = Instant::now();
    send_message(
        process,
        &json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
    )
    .await?;
    let sent_after = started.elapsed();
    let measure_timings = acp_timings_enabled();
    let mut first_text_after = None;
    let timeout = match method {
        "initialize" => Some(Duration::from_secs(120)),
        "authenticate" => Some(Duration::from_secs(300)),
        _ => None,
    };
    let response = if let Some(timeout) = timeout {
        tokio::time::timeout(
            timeout,
            request_response_loop(process, id, method, observer, started, measure_timings, &mut first_text_after),
        )
        .await
        .unwrap_or_else(|_| Err(other(format!("ACP {method} timed out"))))
    } else {
        request_response_loop(process, id, method, observer, started, measure_timings, &mut first_text_after).await
    };
    if measure_timings {
        let elapsed = started.elapsed();
        let status = if method == "session/prompt" {
            match first_text_after {
                Some(first) => format!(
                    "ACP timing session/prompt: send={}ms, first_text={}ms, total={}ms",
                    sent_after.as_millis(), first.as_millis(), elapsed.as_millis()
                ),
                None => format!(
                    "ACP timing session/prompt: send={}ms, first_text=none, total={}ms",
                    sent_after.as_millis(), elapsed.as_millis()
                ),
            }
        } else {
            format!(
                "ACP timing {method}: send={}ms, total={}ms",
                sent_after.as_millis(), elapsed.as_millis()
            )
        };
        let _ = notify(observer, ConversationEvent::Status(status));
    }
    response
}

async fn request_response_loop(
    process: &mut AcpProcess,
    id: u64,
    method: &str,
    observer: &dyn ConversationObserver,
    started: Instant,
    measure_timings: bool,
    first_text_after: &mut Option<Duration>,
) -> Result<Value, ConversationError> {
    loop {
        let message = tokio::select! {
            message = process.messages.recv() => message
                .ok_or_else(|| other("ACP agent closed stdout"))?
                .map_err(other)?,
            auth_url = process.auth_urls.recv() => match auth_url {
                Some(url) => { notify(observer, ConversationEvent::LoginUrl(url))?; continue; },
                None => process.messages.recv().await
                    .ok_or_else(|| other("ACP agent closed stdout"))?
                    .map_err(other)?,
            }
        };
        if message.get("id").and_then(Value::as_u64) == Some(id)
            && message.get("method").is_none()
            && (message.get("result").is_some() || message.get("error").is_some())
        {
            if let Some(error) = message.get("error") {
                return Err(classify_rpc_error(error).unwrap_or_else(|| {
                    other(format!(
                        "ACP {method} request failed: {}",
                        safe_error_message(error)
                    ))
                }));
            }
            return message
                .get("result")
                .cloned()
                .ok_or_else(|| other(format!("ACP {method} response omitted result")));
        }
        if message.get("method").and_then(Value::as_str) == Some("session/update") {
            if measure_timings
                && method == "session/prompt"
                && first_text_after.is_none()
                && message
                    .pointer("/params/update/sessionUpdate")
                    .and_then(Value::as_str)
                    == Some("agent_message_chunk")
                && message
                    .pointer("/params/update/content/text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.is_empty())
            {
                *first_text_after = Some(started.elapsed());
            }
            handle_update(
                Some(process),
                message.get("params").unwrap_or(&Value::Null),
                observer,
            )?;
        } else if message.get("method").and_then(Value::as_str)
            == Some("session/request_permission")
        {
            handle_permission(process, &message, observer).await?;
        } else if let (Some(request_id), Some(_method)) = (
            message.get("id"),
            message.get("method").and_then(Value::as_str),
        ) {
            send_message(process, &json!({"jsonrpc":"2.0","id":request_id,"error":{"code":-32601,"message":"Method not supported by this ACP client"}})).await?;
        }
    }
}

async fn read_protocol_message(
    stdout: &mut (impl AsyncBufRead + Unpin),
    line: &mut Vec<u8>,
) -> Result<Value, ConversationError> {
    if line.capacity() > 64 * 1024 {
        *line = Vec::with_capacity(4096);
    } else {
        line.clear();
    }
    let read = (&mut *stdout)
        .take((MAX_PROTOCOL_LINE_BYTES + 1) as u64)
        .read_until(b'\n', line)
        .await
        .map_err(|e| other(format!("ACP stdout read failed: {e}")))?;
    if read == 0 {
        return Err(other("ACP agent closed stdout"));
    }
    parse_protocol_line(line)
}

fn acp_timings_enabled() -> bool {
    std::env::var("HARNESSMIX_ACP_TIMINGS").as_deref() == Ok("1")
}

fn report_acp_timing(observer: &dyn ConversationObserver, stage: &str, elapsed: Duration) {
    if acp_timings_enabled() {
        let _ = notify(
            observer,
            ConversationEvent::Status(format!("ACP timing {stage}: total={}ms", elapsed.as_millis())),
        );
    }
}

fn parse_protocol_line(line: &[u8]) -> Result<Value, ConversationError> {
    if line.len() > MAX_PROTOCOL_LINE_BYTES {
        return Err(other("ACP message exceeded the 8 MiB limit"));
    }
    if !line.ends_with(b"\n") {
        return Err(other("ACP message was not newline-delimited"));
    }
    serde_json::from_slice(line)
        .map_err(|_| other("ACP agent sent invalid newline-delimited JSON-RPC"))
}

fn classify_rpc_error(error: &Value) -> Option<ConversationError> {
    let numeric_code = error.get("code").and_then(Value::as_i64);
    let data_code = error.pointer("/data/code").and_then(Value::as_str);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("Authentication required");
    if numeric_code == Some(-32000)
        || data_code == Some("auth_required")
        || message.eq_ignore_ascii_case("authentication required")
    {
        Some(ConversationError::AuthenticationRequired(
            message.to_owned(),
        ))
    } else {
        None
    }
}

fn permission_outcome(decision: PermissionDecision, options: &[PermissionOption]) -> Value {
    match decision {
        PermissionDecision::Selected(option_id)
            if options.iter().any(|option| option.id == option_id) =>
        {
            json!({"outcome":{"outcome":"selected","optionId":option_id}})
        }
        PermissionDecision::Selected(_) | PermissionDecision::Cancelled => {
            json!({"outcome":{"outcome":"cancelled"}})
        }
    }
}

fn google_auth_url(line: &str) -> Option<String> {
    let start = line.find("https://accounts.google.com/")?;
    let suffix = &line[start..];
    let end = suffix.find(char::is_whitespace).unwrap_or(suffix.len());
    let url = suffix[..end].trim_matches(|c: char| matches!(c, '"' | '\'' | ')' | ']' | ',' | '.'));
    if url.len() <= MAX_AUTH_URL_BYTES && url.starts_with("https://accounts.google.com/") {
        Some(url.to_owned())
    } else {
        None
    }
}

async fn send_message(process: &mut AcpProcess, message: &Value) -> Result<(), ConversationError> {
    let mut bytes = serde_json::to_vec(message)
        .map_err(|e| other(format!("could not encode ACP message: {e}")))?;
    bytes.push(b'\n');
    let mut stdin = process.stdin.lock().await;
    stdin
        .write_all(&bytes)
        .await
        .map_err(|e| other(format!("ACP stdin write failed: {e}")))?;
    stdin
        .flush()
        .await
        .map_err(|e| other(format!("ACP stdin flush failed: {e}")))
}

fn handle_update(
    process: Option<&mut AcpProcess>,
    params: &Value,
    observer: &dyn ConversationObserver,
) -> Result<(), ConversationError> {
    let update = params.get("update").unwrap_or(&Value::Null);
    let kind = update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .unwrap_or("");
    match kind {
        "agent_message_chunk" => emit_content(update, observer, false),
        "agent_thought_chunk" => emit_content(update, observer, true),
        "tool_call" | "tool_call_update" => notify(
            observer,
            ConversationEvent::ToolCall {
                title: update
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("Agent tool")
                    .to_owned(),
                status: update
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("pending")
                    .to_owned(),
            },
        ),
        "notice" => notify(
            observer,
            ConversationEvent::Status(
                update
                    .get("description")
                    .and_then(Value::as_str)
                    .or_else(|| update.get("title").and_then(Value::as_str))
                    .unwrap_or("Agent notice")
                    .to_owned(),
            ),
        ),
        "plan" => notify(
            observer,
            ConversationEvent::Status("El agente actualizó su plan".to_owned()),
        ),
        "config_option_update" => {
            let Some(raw_options) = update.get("configOptions").filter(|value| value.is_array())
            else {
                return Ok(());
            };
            let config_options = parse_config_options(Some(raw_options));
            if let (Some(process), Some(session_id)) = (
                process,
                params.get("sessionId").and_then(Value::as_str),
            ) {
                process
                    .session_config_options
                    .insert(session_id.to_owned(), config_options.clone());
            }
            notify(
                observer,
                ConversationEvent::ConfigOptions(config_options),
            )
        }
        "usage_update" => match parse_session_usage(update) {
            Some(usage) => notify(observer, ConversationEvent::Usage(usage)),
            None => Ok(()),
        },
        _ => Ok(()),
    }
}

fn emit_content(
    update: &Value,
    observer: &dyn ConversationObserver,
    thought: bool,
) -> Result<(), ConversationError> {
    let text = update
        .pointer("/content/text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    if text.is_empty() {
        return Ok(());
    }
    notify(
        observer,
        if thought {
            ConversationEvent::Thought(text)
        } else {
            ConversationEvent::Text(text)
        },
    )
}

async fn handle_permission(
    process: &mut AcpProcess,
    message: &Value,
    observer: &dyn ConversationObserver,
) -> Result<(), ConversationError> {
    let request_id = message
        .get("id")
        .cloned()
        .ok_or_else(|| other("ACP permission request omitted id"))?;
    let params = message.get("params").unwrap_or(&Value::Null);
    let title = params
        .pointer("/toolCall/title")
        .and_then(Value::as_str)
        .unwrap_or("Agent requests permission")
        .to_owned();
    let options: Vec<PermissionOption> = params
        .get("options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|option| {
            Some(PermissionOption {
                id: option.get("optionId")?.as_str()?.to_owned(),
                name: option.get("name")?.as_str()?.to_owned(),
                kind: option.get("kind")?.as_str()?.to_owned(),
            })
        })
        .collect();
    let decision = match observer.request_permission(PermissionRequest {
        title,
        options: options.clone(),
    }) {
        Ok(decision) => decision,
        Err(error) => {
            send_message(process, &json!({"jsonrpc":"2.0","id":request_id,"result":{"outcome":{"outcome":"cancelled"}}})).await?;
            return Err(other(error));
        }
    };
    let outcome = permission_outcome(decision, &options);
    send_message(
        process,
        &json!({"jsonrpc":"2.0","id":request_id,"result":outcome}),
    )
    .await
}

fn notify(
    observer: &dyn ConversationObserver,
    event: ConversationEvent,
) -> Result<(), ConversationError> {
    observer.on_event(event).map_err(other)
}

fn safe_error_message(error: &Value) -> String {
    // Never surface arbitrary error.data; agents may include sensitive context there.
    error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unspecified protocol error")
        .chars()
        .take(300)
        .collect()
}

fn other(message: impl Into<String>) -> ConversationError {
    ConversationError::Other(message.into())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod line_buffer_tests {
    use super::*;

    #[tokio::test]
    async fn consecutive_protocol_frames_reuse_the_line_buffer() {
        let frames = b"{\"id\":1}\n{\"id\":2}\n";
        let mut reader = BufReader::new(&frames[..]);
        let mut line = Vec::with_capacity(4096);

        assert_eq!(read_protocol_message(&mut reader, &mut line).await.unwrap()["id"], 1);
        let buffer = line.as_ptr();
        assert_eq!(read_protocol_message(&mut reader, &mut line).await.unwrap()["id"], 2);
        assert_eq!(line.as_ptr(), buffer);
    }

    #[tokio::test]
    async fn oversized_protocol_frame_is_rejected_with_a_reused_buffer() {
        let mut oversized = vec![b' '; MAX_PROTOCOL_LINE_BYTES + 1];
        oversized.push(b'\n');
        let mut reader = BufReader::new(&oversized[..]);
        let mut line = Vec::with_capacity(4096);

        assert_eq!(
            read_protocol_message(&mut reader, &mut line).await,
            Err(other("ACP message exceeded the 8 MiB limit"))
        );
    }

    #[tokio::test]
    async fn large_protocol_frame_does_not_keep_a_large_buffer_for_the_next_frame() {
        let large = json!({"data": "x".repeat(70 * 1024)});
        let frames = format!("{large}\n{{\"id\":7}}\n");
        let mut reader = BufReader::new(frames.as_bytes());
        let mut line = Vec::with_capacity(4096);

        assert_eq!(
            read_protocol_message(&mut reader, &mut line).await.unwrap()["data"]
                .as_str()
                .unwrap()
                .len(),
            70 * 1024
        );
        assert!(line.capacity() > 64 * 1024);
        assert_eq!(read_protocol_message(&mut reader, &mut line).await.unwrap()["id"], 7);
        assert!(line.capacity() <= 64 * 1024);
    }
}

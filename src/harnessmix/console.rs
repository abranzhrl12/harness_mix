use crate::{activity::Activity, images, menu, session_settings, rendering::{self, MessageRenderer}};
use futures_util::future::{AbortHandle, Abortable};
use harness_application::conversation::Conversation;
use harness_domain::conversation::{
    AuthMethod, ConnectionInfo, ConversationError, ConversationEvent, ConversationObserver,
    ImageAttachment, PermissionDecision, PermissionPolicy, PermissionRequest, SessionInfo,
    SessionConfigOption, SessionUsage,
};
use rustyline_async::{Readline, ReadlineEvent, SharedWriter};
use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    io::{self, BufRead, IsTerminal, Write},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc as tokio_mpsc;
use tokio::task::JoinHandle;

pub type ConsoleResult<T> = Result<T, String>;

#[derive(Clone, Copy, Debug)]
pub struct ConsoleOptions {
    pub permissions: PermissionPolicy,
    pub animate: bool,
}

impl Default for ConsoleOptions {
    fn default() -> Self {
        Self { permissions: PermissionPolicy::Ask, animate: true }
    }
}

type LocalFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

enum InputEvent { Line(String), Eof, Interrupted }

trait ConsoleInput {
    fn next_event(&mut self) -> LocalFuture<'_, ConsoleResult<InputEvent>>;
    fn update_prompt(&mut self, prompt: &str) -> ConsoleResult<()>;
}

struct TerminalInput(Readline);

impl ConsoleInput for TerminalInput {
    fn next_event(&mut self) -> LocalFuture<'_, ConsoleResult<InputEvent>> {
        Box::pin(async move {
            match self.0.readline().await.map_err(|error| error.to_string())? {
                ReadlineEvent::Line(line) => Ok(InputEvent::Line(line)),
                ReadlineEvent::Eof => Ok(InputEvent::Eof),
                ReadlineEvent::Interrupted => Ok(InputEvent::Interrupted),
            }
        })
    }
    fn update_prompt(&mut self, prompt: &str) -> ConsoleResult<()> {
        self.0.update_prompt(prompt).map_err(|error| error.to_string())
    }
}

struct ChannelInput(tokio_mpsc::UnboundedReceiver<InputEvent>);

impl ConsoleInput for ChannelInput {
    fn next_event(&mut self) -> LocalFuture<'_, ConsoleResult<InputEvent>> {
        Box::pin(async move {
            self.0.recv().await.ok_or_else(|| "console input closed".to_owned())
        })
    }
    fn update_prompt(&mut self, _prompt: &str) -> ConsoleResult<()> { Ok(()) }
}

trait ConsoleOutput {
    fn write_text(&self, text: String) -> LocalFuture<'_, ConsoleResult<()>>;
    fn write_markdown(&self, text: String) -> LocalFuture<'_, ConsoleResult<()>> {
        self.write_text(text)
    }
    fn flush_response(&self) -> LocalFuture<'_, ConsoleResult<()>> {
        Box::pin(async { Ok(()) })
    }
}

/// Presentation decorator: protocol/UI controls remain plain, while assistant
/// chunks are framed into complete blocks before the renderer sees Markdown.
struct RenderingOutput {
    inner: Arc<dyn ConsoleOutput>,
    renderer: Arc<dyn MessageRenderer>,
    pending: Mutex<ResponseStream>,
}

#[derive(Default)]
struct ResponseStream {
    framing: crate::streaming::StreamBuffer,
    ready: VecDeque<String>,
}

impl RenderingOutput {
    fn new(inner: Arc<dyn ConsoleOutput>, renderer: Arc<dyn MessageRenderer>) -> Self {
        Self { inner, renderer, pending: Mutex::new(ResponseStream::default()) }
    }

    async fn emit_ready(&self) -> ConsoleResult<()> {
        loop {
            let markdown = self.pending.lock().map_err(|_| "response buffer unavailable")?
                .ready.pop_front();
            let Some(markdown) = markdown else { return Ok(()); };
            let mut rendered = self.renderer.render(&markdown, rendering::terminal_width());
            // Only complete output lines reach SharedWriter, avoiding its partial
            // token-flush cursor bookkeeping while preserving the editable prompt.
            if !rendered.ends_with('\n') { rendered.push('\n'); }
            if let Err(error) = self.inner.write_text(rendered).await {
                self.pending.lock().map_err(|_| "response buffer unavailable")?
                    .ready.push_front(markdown);
                return Err(error);
            }
        }
    }
}

impl ConsoleOutput for RenderingOutput {
    fn write_markdown(&self, text: String) -> LocalFuture<'_, ConsoleResult<()>> {
        Box::pin(async move {
            {
                let mut pending = self.pending.lock().map_err(|_| "response buffer unavailable")?;
                let segments = pending.framing.push(&text);
                pending.ready.extend(segments);
            }
            self.emit_ready().await
        })
    }

    fn write_text(&self, text: String) -> LocalFuture<'_, ConsoleResult<()>> {
        self.inner.write_text(rendering::sanitize(&text))
    }

    fn flush_response(&self) -> LocalFuture<'_, ConsoleResult<()>> {
        Box::pin(async move {
            {
                let mut pending = self.pending.lock().map_err(|_| "response buffer unavailable")?;
                if let Some(tail) = pending.framing.finish() { pending.ready.push_back(tail); }
            }
            self.emit_ready().await
        })
    }
}

struct PlainOutput<W>(Arc<Mutex<W>>);

impl<W: Write + Send> ConsoleOutput for PlainOutput<W> {
    fn write_text(&self, text: String) -> LocalFuture<'_, ConsoleResult<()>> {
        Box::pin(async move {
            let mut output = self.0.lock().map_err(|_| "console output unavailable")?;
            output.write_all(text.as_bytes()).and_then(|_| output.flush())
                .map_err(|error| error.to_string())
        })
    }
}

struct TerminalOutput(tokio::sync::Mutex<SharedWriter>);

impl ConsoleOutput for TerminalOutput {
    fn write_text(&self, text: String) -> LocalFuture<'_, ConsoleResult<()>> {
        Box::pin(async move {
            let mut output = self.0.lock().await;
            futures_util::io::AsyncWriteExt::write_all(&mut *output, text.as_bytes())
                .await.map_err(|error| error.to_string())?;
            futures_util::io::AsyncWriteExt::flush(&mut *output)
                .await.map_err(|error| error.to_string())
        })
    }
}

enum UiEvent {
    Metadata { event: ConversationEvent, ack: mpsc::SyncSender<ConsoleResult<()>> },
    Output { text: String, markdown: bool, ack: mpsc::SyncSender<ConsoleResult<()>> },
    Permission { request: PermissionRequest, reply: mpsc::SyncSender<PermissionDecision> },
}

#[derive(Clone)]
struct ConsoleObserver {
    events: tokio_mpsc::UnboundedSender<UiEvent>,
    permissions: Arc<Mutex<PermissionPolicy>>,
    activity: Arc<Activity>,
}

impl ConversationObserver for ConsoleObserver {
    fn on_event(&self, event: ConversationEvent) -> Result<(), String> {
        if matches!(&event, ConversationEvent::ConfigOptions(_) | ConversationEvent::Usage(_)) {
            let (ack, received) = mpsc::sync_channel(1);
            self.events.send(UiEvent::Metadata { event, ack })
                .map_err(|_| "console event loop closed".to_owned())?;
            return received.recv().map_err(|_| "console metadata acknowledgement closed".to_owned())?;
        }
        let markdown = matches!(&event, ConversationEvent::Text(_));
        let Some(text) = event_text(event) else { return Ok(()); };
        let resume = self.activity.pause()?;
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        if self.events.send(UiEvent::Output { text, markdown, ack: ack_tx }).is_err() {
            self.activity.resume(resume)?;
            return Err("console event loop closed".into());
        }
        let result = ack_rx.recv().map_err(|_| "console output acknowledgement closed".to_owned())?;
        self.activity.resume(resume)?;
        result
    }

    fn request_permission(&self, request: PermissionRequest) -> Result<PermissionDecision, String> {
        let resume = self.activity.pause()?;
        let policy = *self.permissions.lock().map_err(|_| "permission policy unavailable")?;
        if let Some(decision) = policy.decision(&request) {
            let (ack_tx, ack_rx) = mpsc::sync_channel(1);
            if self.events.send(UiEvent::Output {
                text: format!("\n{}\n", permission_action(&request, &decision)),
                markdown: false,
                ack: ack_tx,
            }).is_err() {
                self.activity.resume(resume)?;
                return Err("console event loop closed".into());
            }
            let result = ack_rx.recv().map_err(|_| "console output acknowledgement closed".to_owned())?;
            self.activity.resume(resume)?;
            result?;
            return Ok(decision);
        }
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        if self.events.send(UiEvent::Permission { request, reply: reply_tx }).is_err() {
            self.activity.resume(resume)?;
            return Err("console event loop closed".into());
        }
        let decision = reply_rx.recv().map_err(|_| "permission prompt closed".to_owned())?;
        self.activity.resume(resume)?;
        Ok(decision)
    }
}

fn event_text(event: ConversationEvent) -> Option<String> {
    match event {
        ConversationEvent::ConfigOptions(_) | ConversationEvent::Usage(_) => None,
        ConversationEvent::Text(text) => Some(text),
        ConversationEvent::Thought(_) => None,
        ConversationEvent::ToolCall { title, status } => {
            let label = match status.as_str() {
                "pending" => "Pendiente", "in_progress" => "En curso",
                "completed" => "Listo", "failed" => "Error", _ => "Herramienta",
            };
            let title = if title == "Agent tool" { "Herramienta" } else { &title };
            Some(format!("\n[{label}] {title}\n"))
        }
        ConversationEvent::Status(status) => Some(format!("\n{status}\n")),
        ConversationEvent::LoginUrl(url) => {
            if url.to_ascii_lowercase().contains("continue=https%3a%2f%2fone.google.com%2fai") {
                None
            } else {
                Some(format!("\nAbre esta dirección para iniciar sesión con Google:\n{url}\n"))
            }
        }
    }
}

fn permission_action(request: &PermissionRequest, decision: &PermissionDecision) -> String {
    match decision {
        PermissionDecision::Selected(id) => {
            let action = request.options.iter().find(|item| &item.id == id)
                .map(|item| item.name.as_str()).unwrap_or("acción aprobada");
            format!("Permiso aprobado automáticamente: {action}.")
        }
        PermissionDecision::Cancelled =>
            "Permiso denegado automáticamente; no hay opción de aprobación disponible.".into(),
    }
}

#[derive(Default)]
struct UiState {
    connection: Option<ConnectionInfo>,
    session_id: Option<String>,
    config_options: Vec<SessionConfigOption>,
    usage: Option<SessionUsage>,
    pending_lines: VecDeque<String>,
    pending_prompts: VecDeque<PendingPrompt>,
    next_image_ids: Vec<u64>,
    image_results: HashMap<u64, Result<ImageAttachment, String>>,
    next_image_id: u64,
    menu_mode: bool,
    discard_menu_choices: bool,
    exit_requested: bool,
    cancel_requested: bool,
}

struct PendingPrompt { text: String, image_ids: Vec<u64> }
type ImageCompletion = (u64, Result<ImageAttachment, String>);

struct UiChannels {
    events: tokio_mpsc::UnboundedReceiver<UiEvent>,
    image_tx: tokio_mpsc::UnboundedSender<ImageCompletion>,
    image_rx: tokio_mpsc::UnboundedReceiver<ImageCompletion>,
}

pub async fn run_with_options<P>(
    conversation: Arc<Conversation<P>>,
    workspace: &Path,
    options: ConsoleOptions,
) -> ConsoleResult<()>
where P: harness_application::ports::conversation::ConversationProvider + 'static {
    let stdin = io::stdin();
    let stdout = io::stdout();
    if stdin.is_terminal() && stdout.is_terminal() {
        let (readline, writer) = Readline::new(menu::CHAT_PROMPT.to_owned())
            .map_err(|error| error.to_string())?;
        run_loop(
            conversation, workspace.to_path_buf(), Box::new(TerminalInput(readline)),
            Arc::new(TerminalOutput(tokio::sync::Mutex::new(writer))),
            options, true,
        ).await
    } else {
        run_with_io_options(conversation, workspace, io::BufReader::new(stdin), stdout, options).await
    }
}

#[cfg(test)]
pub async fn run_with_io<P, R, W>(
    conversation: Arc<Conversation<P>>,
    workspace: &Path,
    input: R,
    output: W,
) -> ConsoleResult<()>
where
    P: harness_application::ports::conversation::ConversationProvider + 'static,
    R: BufRead + Send + 'static,
    W: Write + Send + 'static,
{
    run_with_io_options(
        conversation, workspace, input, output,
        ConsoleOptions { permissions: PermissionPolicy::Ask, animate: false },
    ).await
}

pub async fn run_with_io_options<P, R, W>(
    conversation: Arc<Conversation<P>>,
    workspace: &Path,
    input: R,
    output: W,
    options: ConsoleOptions,
) -> ConsoleResult<()>
where
    P: harness_application::ports::conversation::ConversationProvider + 'static,
    R: BufRead + Send + 'static,
    W: Write + Send + 'static,
{
    let (sender, receiver) = tokio_mpsc::unbounded_channel();
    std::thread::spawn(move || input_thread(input, sender));
    run_loop(
        conversation, workspace.to_path_buf(), Box::new(ChannelInput(receiver)),
        Arc::new(PlainOutput(Arc::new(Mutex::new(output)))),
        options, false,
    ).await
}

fn input_thread<R: BufRead>(mut input: R, sender: tokio_mpsc::UnboundedSender<InputEvent>) {
    loop {
        let mut line = String::new();
        match input.read_line(&mut line) {
            Ok(0) => { let _ = sender.send(InputEvent::Eof); break; }
            Ok(_) => {
                let line = line.trim_end_matches(['\r', '\n']).to_owned();
                if sender.send(InputEvent::Line(line)).is_err() { break; }
            }
            Err(_) => { let _ = sender.send(InputEvent::Eof); break; }
        }
    }
}

async fn run_loop<P>(
    conversation: Arc<Conversation<P>>,
    workspace: PathBuf,
    mut input: Box<dyn ConsoleInput>,
    output: Arc<dyn ConsoleOutput>,
    options: ConsoleOptions,
    terminal: bool,
) -> ConsoleResult<()>
where P: harness_application::ports::conversation::ConversationProvider + 'static {
    let styled = terminal && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").map_or(true, |value| value != "dumb");
    let output: Arc<dyn ConsoleOutput> = Arc::new(RenderingOutput::new(output, rendering::renderer(styled)));
    let (event_tx, event_rx) = tokio_mpsc::unbounded_channel();
    let (image_tx, image_rx) = tokio_mpsc::unbounded_channel();
    let mut channels = UiChannels { events: event_rx, image_tx, image_rx };
    let policy = Arc::new(Mutex::new(options.permissions));
    let activity = Arc::new(Activity::new(terminal && options.animate));
    let observer = Arc::new(ConsoleObserver {
        events: event_tx, permissions: policy.clone(), activity: activity.clone(),
    });
    let mut state = UiState::default();

    output.write_text(format!("{}\n", menu::render_startup_banner())).await?;
    input.update_prompt(menu::CHAT_PROMPT)?;

    activity.start("Conectando con Antigravity…")?;
    let connect = spawn_operation({
        let conversation = conversation.clone();
        let observer = observer.clone();
        async move { conversation.connect(observer.as_ref()).await }
    });
    state.connection = Some(wait_setup(
        connect, &mut *input, &output, &mut channels, &activity, &policy, &mut state,
    ).await?);

    state.session_id = open_session(
        &conversation, &workspace, &observer, &mut *input, &output, &mut channels,
        &activity, &policy, &mut state,
    ).await?;

    let mut interval = tokio::time::interval(Duration::from_millis(100));
    loop {
        if state.exit_requested { break; }

        // Apply a model/effort change queued during the previous turn before
        // dispatching the next queued user prompt.
        if state.pending_lines.front().is_some_and(|line| is_setting_command(line)) {
            let line = state.pending_lines.pop_front().unwrap();
            process_idle_line(&line, &conversation, &workspace, &observer, &mut *input,
                &output, &mut channels, &activity, &policy, &mut state).await?;
            continue;
        }

        if let Some(prompt) = ready_prompt(&mut state) {
            if state.session_id.is_none() {
                output.write_text("No hay una sesión activa. Usa /new o /login.\n".into()).await?;
                continue;
            }
            let images = match take_images(&mut state, &prompt.image_ids) {
                Ok(images) => images,
                Err(error) => { output.write_text(format!("{error}\n")).await?; continue; }
            };
            let session_id = state.session_id.clone().unwrap();
            activity.start("Pensando y escribiendo…")?;
            let mut send_task = spawn_operation({
                let conversation = conversation.clone();
                let observer = observer.clone();
                async move {
                    conversation.send_with_images(
                        &session_id, &prompt.text, &images, observer.as_ref(),
                    ).await
                }
            });
            let (cancel_tx, mut cancel_rx) = tokio_mpsc::unbounded_channel();
            let mut cancel_started = false;
            let mut cancel_deadline: Option<tokio::time::Instant> = None;

            loop {
                tokio::select! {
                    event = channels.events.recv() => {
                        if let Some(event) = event {
                            let outcome = dispatch_event(
                                event, &mut *input, &output, &activity, &policy,
                                &mut state, &mut channels,
                            ).await?;
                            if outcome && !cancel_started {
                                start_cancel(&conversation, &state, &cancel_tx, &mut cancel_started, &mut cancel_deadline);
                            }
                        }
                    }
                    result = &mut send_task => {
                        activity.stop()?;
                        input.update_prompt(current_prompt(&state))?;
                        output.flush_response().await?;
                        match result {
                            Ok(Ok(_)) => output.write_text("\n".into()).await?,
                            Ok(Err(error)) => output.write_text(format!("\n{error}\n")).await?,
                            Err(error) => output.write_text(format!("\nError del turno: {error}\n")).await?,
                        }
                        if cancel_started { output.write_text("Generación cancelada.\n".into()).await?; }
                        break;
                    }
                    line = input.next_event() => {
                        match line? {
                            InputEvent::Line(line) => handle_live_line(
                                &line, &conversation, &mut *input, &output, &activity, &policy,
                                &mut state, &mut channels, &cancel_tx, &mut cancel_started,
                                &mut cancel_deadline,
                            ).await?,
                            InputEvent::Interrupted | InputEvent::Eof => {
                                state.exit_requested = true;
                                start_cancel(&conversation, &state, &cancel_tx, &mut cancel_started, &mut cancel_deadline);
                            }
                        }
                    }
                    image = channels.image_rx.recv() => {
                        if let Some(image) = image { finish_image(image, &output, &mut state).await?; }
                    }
                    cancel = cancel_rx.recv(), if cancel_started => {
                        if let Some(Err(error)) = cancel {
                            output.write_text(format!("No se confirmó la cancelación: {error}\n")).await?;
                        }
                    }
                    _ = interval.tick() => render_activity(&mut *input, &activity, &state)?,
                    _ = wait_until(cancel_deadline), if cancel_started => {
                        send_task.abort();
                        let _ = send_task.await;
                        activity.stop()?;
                        output.flush_response().await?;
                        output.write_text("El agente no respondió a la cancelación; se cerrará la sesión.\n".into()).await?;
                        state.exit_requested = true;
                        break;
                    }
                    signal = tokio::signal::ctrl_c(), if !cancel_started => {
                        signal.map_err(|error| error.to_string())?;
                        state.exit_requested = true;
                        start_cancel(&conversation, &state, &cancel_tx, &mut cancel_started, &mut cancel_deadline);
                    }
                }
            }
            continue;
        }

        if let Some(line) = state.pending_lines.pop_front() {
            process_idle_line(
                &line, &conversation, &workspace, &observer, &mut *input, &output,
                &mut channels, &activity, &policy, &mut state,
            ).await?;
            continue;
        }

        tokio::select! {
            event = channels.events.recv() => {
                if let Some(event) = event {
                    let _ = dispatch_event(
                        event, &mut *input, &output, &activity, &policy,
                        &mut state, &mut channels,
                    ).await?;
                }
            }
            line = input.next_event() => {
                match line? {
                    InputEvent::Line(line) => process_idle_line(
                        &line, &conversation, &workspace, &observer, &mut *input, &output,
                        &mut channels, &activity, &policy, &mut state,
                    ).await?,
                    InputEvent::Interrupted | InputEvent::Eof => state.exit_requested = true,
                }
            }
            image = channels.image_rx.recv() => {
                if let Some(image) = image { finish_image(image, &output, &mut state).await?; }
            }
            _ = interval.tick() => render_activity(&mut *input, &activity, &state)?,
            signal = tokio::signal::ctrl_c() => {
                signal.map_err(|error| error.to_string())?;
                state.exit_requested = true;
            }
        }
    }

    activity.stop()?;
    output.flush_response().await?;
    conversation.close().await.map_err(|error| error.to_string())
}

struct OperationHandle<T> {
    task: JoinHandle<Result<Result<T, ConversationError>, futures_util::future::Aborted>>,
    abort: AbortHandle,
}

impl<T> OperationHandle<T> {
    fn abort(&self) {
        self.abort.abort();
    }
}

impl<T> Future for OperationHandle<T> {
    type Output = Result<Result<T, ConversationError>, String>;

    fn poll(
        mut self: Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        match Pin::new(&mut self.task).poll(context) {
            std::task::Poll::Pending => std::task::Poll::Pending,
            std::task::Poll::Ready(Ok(Ok(result))) => std::task::Poll::Ready(Ok(result)),
            std::task::Poll::Ready(Ok(Err(_))) => {
                std::task::Poll::Ready(Err("Operación cancelada.".into()))
            }
            std::task::Poll::Ready(Err(error)) => {
                std::task::Poll::Ready(Err(error.to_string()))
            }
        }
    }
}

fn spawn_operation<T, F>(future: F) -> OperationHandle<T>
where
    T: Send + 'static,
    F: Future<Output = Result<T, ConversationError>> + Send + 'static,
{
    let runtime = tokio::runtime::Handle::current();
    let (abort, registration) = AbortHandle::new_pair();
    let task = tokio::task::spawn_blocking(move || {
        runtime.block_on(Abortable::new(future, registration))
    });
    OperationHandle { task, abort }
}

async fn wait_setup<T, I>(
    mut task: OperationHandle<T>,
    input: &mut I,
    output: &Arc<dyn ConsoleOutput>,
    channels: &mut UiChannels,
    activity: &Arc<Activity>,
    policy: &Arc<Mutex<PermissionPolicy>>,
    state: &mut UiState,
) -> ConsoleResult<T>
where I: ConsoleInput + ?Sized, T: Send + 'static {
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            result = &mut task => {
                activity.stop()?;
                input.update_prompt(current_prompt(state))?;
                return result.map_err(|error| error.to_string())?.map_err(|error| match error {
                    ConversationError::AuthenticationRequired(message) => format!("Se requiere iniciar sesión: {message}"),
                    ConversationError::Other(message) => message,
                });
            }
            event = channels.events.recv() => {
                if let Some(event) = event {
                    let _ = dispatch_event(event, input, output, activity, policy, state, channels).await?;
                }
            }
            line = input.next_event() => match line? {
                InputEvent::Line(line) => {
                    if !(state.discard_menu_choices && line.trim().parse::<usize>().is_ok()) {
                        state.pending_lines.push_back(line);
                    }
                }
                InputEvent::Interrupted | InputEvent::Eof => {
                    task.abort();
                    let _ = task.await;
                    activity.stop()?;
                    return Err("Operación interrumpida.".into());
                }
            },
            image = channels.image_rx.recv() => {
                if let Some(image) = image { finish_image(image, output, state).await?; }
            }
            _ = interval.tick() => render_activity(input, activity, state)?,
            signal = tokio::signal::ctrl_c() => {
                signal.map_err(|error| error.to_string())?;
                task.abort();
                let _ = task.await;
                activity.stop()?;
                return Err("Operación interrumpida.".into());
            }
        }
    }
}

async fn open_session<P, I>(
    conversation: &Arc<Conversation<P>>,
    workspace: &Path,
    observer: &Arc<ConsoleObserver>,
    input: &mut I,
    output: &Arc<dyn ConsoleOutput>,
    channels: &mut UiChannels,
    activity: &Arc<Activity>,
    policy: &Arc<Mutex<PermissionPolicy>>,
    state: &mut UiState,
) -> ConsoleResult<Option<String>>
where P: harness_application::ports::conversation::ConversationProvider + 'static,
      I: ConsoleInput + ?Sized {
    activity.start("Abriendo una sesión…")?;
    state.config_options.clear();
    state.usage = None;
    let task = spawn_operation({
        let conversation = conversation.clone();
        let workspace = workspace.to_path_buf();
        let observer = observer.clone();
        async move { conversation.start_session(&workspace, observer.as_ref()).await }
    });
    match wait_setup(task, input, output, channels, activity, policy, state).await {
        Ok(SessionInfo { id, config_options }) => {
            state.config_options = config_options;
            Ok(Some(id))
        },
        Err(message) if message == "Operación interrumpida." => Err(message),
        Err(message) if message.starts_with("Se requiere iniciar sesión:") => {
            output.write_text(format!("{message}. Usa /login.\n")).await?;
            Ok(None)
        }
        Err(message) => {
            output.write_text(format!(
                "No se pudo abrir una sesión nueva: {message}. Reintenta con /new; si hace falta, usa /login.\n"
            )).await?;
            Ok(None)
        }
    }
}

async fn process_idle_line<P, I>(
    line: &str,
    conversation: &Arc<Conversation<P>>,
    workspace: &Path,
    observer: &Arc<ConsoleObserver>,
    input: &mut I,
    output: &Arc<dyn ConsoleOutput>,
    channels: &mut UiChannels,
    activity: &Arc<Activity>,
    policy: &Arc<Mutex<PermissionPolicy>>,
    state: &mut UiState,
) -> ConsoleResult<()>
where P: harness_application::ports::conversation::ConversationProvider + 'static,
      I: ConsoleInput + ?Sized {
    if state.menu_mode {
        return handle_menu_choice(
            line, false, conversation, workspace, observer, input, output,
            channels, activity, policy, state,
        ).await;
    }
    let trimmed = line.trim();
    match trimmed {
        "" => Ok(()),
        "/menu" => {
            state.menu_mode = true;
            output.write_text(menu::render_menu().into()).await?;
            input.update_prompt(menu::MENU_PROMPT)
        }
        "/exit" | "/quit" => { state.exit_requested = true; Ok(()) }
        "/config" => output.write_text(config_text(state, *policy.lock().map_err(|_| "permission policy unavailable")?)).await,
        "/usage" => output.write_text(session_settings::usage(state.usage.as_ref())).await,
        _ if is_setting_command(trimmed) => change_session_setting(trimmed, conversation, observer, input, output, channels, activity, policy, state).await,
        "/permissions" => show_permissions(output, policy).await,
        "/permissions auto" => set_permissions(output, policy, PermissionPolicy::AutoApprove).await,
        "/permissions ask" => set_permissions(output, policy, PermissionPolicy::Ask).await,
        "/cancel" => output.write_text("No hay una respuesta en curso para cancelar.\n".into()).await,
        "/new" => {
            state.session_id = open_session(conversation, workspace, observer, input, output, channels, activity, policy, state).await?;
            if state.session_id.is_some() { output.write_text("Nueva sesión iniciada.\n".into()).await?; }
            Ok(())
        }
        "/login" => perform_login(conversation, workspace, observer, input, output, channels, activity, policy, state).await,
        _ if trimmed.starts_with("/image") => {
            if let Some(path) = image_path(line) { enqueue_image(path, state, &channels.image_tx, output).await }
            else { output.write_text("Uso: /image <ruta> (entre comillas si contiene espacios).\n".into()).await }
        }
        _ if trimmed.starts_with('/') => output.write_text(
            "Comandos: /menu, /login, /new, /model [opción], /effort [opción], /usage, /config, /permissions [auto|ask], /image <ruta>, /exit\n".into()
        ).await,
        _ if state.session_id.is_none() => output.write_text("No hay una sesión activa. Usa /new o /login.\n".into()).await,
        _ => { queue_prompt(line.to_owned(), state); Ok(()) }
    }
}

fn is_setting_command(line: &str) -> bool {
    matches!(line.split_whitespace().next(), Some("/model" | "/effort"))
}

async fn change_session_setting<P, I>(
    line: &str, conversation: &Arc<Conversation<P>>, observer: &Arc<ConsoleObserver>,
    input: &mut I, output: &Arc<dyn ConsoleOutput>, channels: &mut UiChannels,
    activity: &Arc<Activity>, policy: &Arc<Mutex<PermissionPolicy>>, state: &mut UiState,
) -> ConsoleResult<()>
where P: harness_application::ports::conversation::ConversationProvider + 'static,
      I: ConsoleInput + ?Sized {
    let mut words = line.split_whitespace();
    let command = words.next().unwrap_or("/model");
    let Some(value) = words.next() else {
        return output.write_text(session_settings::list(&state.config_options, command)).await;
    };
    if words.next().is_some() {
        return output.write_text(format!("Uso: {command} <número o id>.\n")).await;
    }
    let Some(option) = session_settings::selector(&state.config_options, command) else {
        return output.write_text(session_settings::list(&state.config_options, command)).await;
    };
    let selected = match session_settings::selected_value(option, value) {
        Ok(value) => value.to_owned(),
        Err(message) => return output.write_text(message).await,
    };
    let config_id = option.id.clone();
    let Some(session_id) = state.session_id.clone() else {
        return output.write_text("No hay una sesión activa. Usa /new.\n".into()).await;
    };
    activity.start("Actualizando configuración…")?;
    let task = spawn_operation({
        let conversation = conversation.clone();
        let observer = observer.clone();
        async move { conversation.set_config_option(&session_id, &config_id, &selected, observer.as_ref()).await }
    });
    match wait_setup(task, input, output, channels, activity, policy, state).await {
        Ok(options) => {
            state.config_options = options;
            output.write_text(session_settings::list(&state.config_options, command)).await
        }
        Err(message) => output.write_text(format!("No se pudo cambiar la configuración: {message}\n")).await,
    }
}

async fn perform_login<P, I>(
    conversation: &Arc<Conversation<P>>,
    workspace: &Path,
    observer: &Arc<ConsoleObserver>,
    input: &mut I,
    output: &Arc<dyn ConsoleOutput>,
    channels: &mut UiChannels,
    activity: &Arc<Activity>,
    policy: &Arc<Mutex<PermissionPolicy>>,
    state: &mut UiState,
) -> ConsoleResult<()>
where P: harness_application::ports::conversation::ConversationProvider + 'static,
      I: ConsoleInput + ?Sized {
    let Some(auth_methods) = state.connection.as_ref().map(|connection| connection.auth_methods.clone()) else {
        output.write_text("Todavía no hay conexión ACP.\n".into()).await?;
        return Ok(());
    };
    let was_menu = state.menu_mode;
    state.menu_mode = false;
    if was_menu { input.update_prompt(menu::CHAT_PROMPT)?; }
    state.discard_menu_choices = was_menu;
    let Some(method) = choose_auth_method(&auth_methods, input, output, state).await? else {
        state.discard_menu_choices = false;
        output.write_text("Inicio de sesión cancelado.\n".into()).await?;
        return Ok(());
    };
    input.update_prompt(menu::CHAT_PROMPT)?;
    activity.start("Iniciando sesión…")?;
    let task = spawn_operation({
        let conversation = conversation.clone();
        let observer = observer.clone();
        let method_id = method.id;
        async move { conversation.authenticate(&method_id, observer.as_ref()).await }
    });
    match wait_setup(task, input, output, channels, activity, policy, state).await {
        Ok(connection) => {
            state.connection = Some(connection);
            state.session_id = open_session(conversation, workspace, observer, input, output, channels, activity, policy, state).await?;
            if state.session_id.is_some() { output.write_text("Sesión iniciada.\n".into()).await?; }
        }
        Err(error) => output.write_text(format!("Error de autenticación: {error}\n")).await?,
    }
    state.discard_menu_choices = false;
    if was_menu { state.pending_lines.retain(|line| line.trim().parse::<usize>().is_err()); }
    Ok(())
}

async fn choose_auth_method<I: ConsoleInput + ?Sized>(
    methods: &[AuthMethod],
    input: &mut I,
    output: &Arc<dyn ConsoleOutput>,
    state: &mut UiState,
) -> ConsoleResult<Option<AuthMethod>> {
    if methods.is_empty() {
        output.write_text("El agente no anunció métodos de autenticación.\n".into()).await?;
        return Ok(None);
    }
    let mut text = "Métodos disponibles:\n".to_owned();
    for (index, method) in methods.iter().enumerate() {
        text.push_str(&format!("{}. {}\n", index + 1, method.name));
        if let Some(description) = &method.description { text.push_str(&format!("   {description}\n")); }
    }
    text.push_str("0. Cancelar\n");
    output.write_text(text).await?;
    loop {
        input.update_prompt("Método de inicio de sesión > ")?;
        let line = if let Some(line) = state.pending_lines.pop_front() {
            InputEvent::Line(line)
        } else { input.next_event().await? };
        match line {
            InputEvent::Line(line) => {
                let Ok(choice) = line.trim().parse::<usize>() else {
                    state.pending_lines.push_front(line);
                    return Ok(None);
                };
                return Ok(choice.checked_sub(1).and_then(|index| methods.get(index)).cloned());
            }
            InputEvent::Interrupted | InputEvent::Eof => return Ok(None),
        }
    }
}

async fn handle_menu_choice<P, I>(
    line: &str, active: bool, conversation: &Arc<Conversation<P>>, workspace: &Path,
    observer: &Arc<ConsoleObserver>, input: &mut I, output: &Arc<dyn ConsoleOutput>,
    channels: &mut UiChannels, activity: &Arc<Activity>, policy: &Arc<Mutex<PermissionPolicy>>,
    state: &mut UiState,
) -> ConsoleResult<()>
where P: harness_application::ports::conversation::ConversationProvider + 'static,
      I: ConsoleInput + ?Sized {
    use menu::MenuAction;
    match menu::parse_menu_choice(line) {
        MenuAction::BackToChat | MenuAction::Cancel => {
            state.menu_mode = false;
            input.update_prompt(menu::CHAT_PROMPT)
        }
        MenuAction::InvalidChoice if line.trim().parse::<usize>().is_err() => {
            state.menu_mode = false;
            input.update_prompt(menu::CHAT_PROMPT)?;
            Box::pin(process_idle_line(line, conversation, workspace, observer, input, output, channels,
                activity, policy, state)).await
        }
        MenuAction::InvalidChoice => output.write_text("Elige una opción del 0 al 5.\n".into()).await,
        MenuAction::Exit => { state.exit_requested = true; Ok(()) }
        MenuAction::Config => output.write_text(config_text(state, *policy.lock().map_err(|_| "permission policy unavailable")?)).await,
        MenuAction::Permissions => {
            show_permissions(output, policy).await?;
            output.write_text("Usa /permissions auto o /permissions ask.\n".into()).await
        }
        MenuAction::Login if active => {
            state.menu_mode = false;
            input.update_prompt(menu::CHAT_PROMPT)?;
            state.pending_lines.push_back("/login".into());
            output.write_text("Inicio de sesión en cola para después del turno.\n".into()).await
        }
        MenuAction::NewSession if active => {
            state.menu_mode = false;
            input.update_prompt(menu::CHAT_PROMPT)?;
            state.pending_lines.push_back("/new".into());
            output.write_text("Nueva sesión en cola para después del turno.\n".into()).await
        }
        MenuAction::Login => perform_login(conversation, workspace, observer, input, output, channels, activity, policy, state).await,
        MenuAction::NewSession => {
            state.menu_mode = false;
            state.discard_menu_choices = true;
            input.update_prompt(menu::CHAT_PROMPT)?;
            state.session_id = open_session(conversation, workspace, observer, input, output, channels, activity, policy, state).await?;
            if state.session_id.is_some() { output.write_text("Nueva sesión iniciada.\n".into()).await?; }
            state.discard_menu_choices = false;
            state.pending_lines.retain(|line| line.trim().parse::<usize>().is_err());
            Ok(())
        }
    }
}

async fn handle_live_line<P, I>(
    line: &str, conversation: &Arc<Conversation<P>>, input: &mut I,
    output: &Arc<dyn ConsoleOutput>, activity: &Arc<Activity>,
    policy: &Arc<Mutex<PermissionPolicy>>, state: &mut UiState, channels: &mut UiChannels,
    cancel_tx: &tokio_mpsc::UnboundedSender<Result<(), ConversationError>>,
    cancel_started: &mut bool, cancel_deadline: &mut Option<tokio::time::Instant>,
) -> ConsoleResult<()>
where P: harness_application::ports::conversation::ConversationProvider + 'static,
      I: ConsoleInput + ?Sized {
    if state.menu_mode {
        // Parsing is restricted to menu mode so chat input like "1" stays text.
        return match menu::parse_menu_choice(line) {
            menu::MenuAction::BackToChat | menu::MenuAction::Cancel => {
                state.menu_mode = false;
                input.update_prompt(menu::CHAT_PROMPT)
            }
            menu::MenuAction::InvalidChoice if line.trim().parse::<usize>().is_err() => {
                state.menu_mode = false;
                input.update_prompt(menu::CHAT_PROMPT)?;
                Box::pin(handle_live_line(
                    line, conversation, input, output, activity, policy, state,
                    channels, cancel_tx, cancel_started, cancel_deadline,
                )).await
            }
            menu::MenuAction::InvalidChoice => output.write_text("Elige una opción del 0 al 5.\n".into()).await,
            menu::MenuAction::Exit => {
                state.exit_requested = true;
                begin_cancel(conversation, state, cancel_tx, cancel_started, cancel_deadline, activity, input)?;
                Ok(())
            }
            menu::MenuAction::Permissions => show_permissions(output, policy).await,
            menu::MenuAction::Config => output.write_text(config_text(state, *policy.lock().map_err(|_| "permission policy unavailable")?)).await,
            menu::MenuAction::Login | menu::MenuAction::NewSession => {
                state.menu_mode = false;
                input.update_prompt(menu::CHAT_PROMPT)?;
                state.pending_lines.push_back(if line.trim() == "1" { "/login" } else { "/new" }.into());
                output.write_text("Comando en cola.\n".into()).await
            }
        };
    }
    let trimmed = line.trim();
    match trimmed {
        "/usage" => output.write_text(session_settings::usage(state.usage.as_ref())).await,
        _ if is_setting_command(trimmed) => {
            if trimmed.split_whitespace().count() == 1 {
                output.write_text(session_settings::list(&state.config_options, trimmed)).await
            } else {
                state.pending_lines.push_back(trimmed.to_owned());
                output.write_text("Cambio de configuración en cola para después del turno.\n".into()).await
            }
        }
        "/menu" => {
            state.menu_mode = true;
            input.update_prompt(menu::MENU_PROMPT)?;
            output.write_text(menu::render_menu().into()).await
        }
        "/permissions" => show_permissions(output, policy).await,
        "/permissions auto" => set_permissions(output, policy, PermissionPolicy::AutoApprove).await,
        "/permissions ask" => set_permissions(output, policy, PermissionPolicy::Ask).await,
        "/config" => output.write_text(config_text(state, *policy.lock().map_err(|_| "permission policy unavailable")?)).await,
        "/cancel" | "/exit" | "/quit" => {
            state.exit_requested = trimmed != "/cancel";
            begin_cancel(conversation, state, cancel_tx, cancel_started, cancel_deadline, activity, input)?;
            output.write_text(if state.exit_requested {
                "Cerrando después de cancelar el turno actual…\n".into()
            } else { "Cancelación solicitada.\n".into() }).await
        }
        "/login" | "/new" => {
            state.pending_lines.push_back(trimmed.to_owned());
            output.write_text("Comando en cola para después del turno.\n".into()).await
        }
        _ if trimmed.starts_with("/image") => {
            if let Some(path) = image_path(line) {
                enqueue_image(path, state, &channels.image_tx, output).await
            } else { output.write_text("Uso: /image <ruta>.\n".into()).await }
        }
        _ if trimmed.starts_with('/') => output.write_text("Comandos: /menu, /permissions [auto|ask], /config, /image <ruta>, /cancel, /exit\n".into()).await,
        _ => {
            queue_prompt(line.to_owned(), state);
            output.write_text(format!("Mensaje en cola (#{}).\n", state.pending_prompts.len())).await
        }
    }
}

fn begin_cancel<P: harness_application::ports::conversation::ConversationProvider + 'static, I: ConsoleInput + ?Sized>(
    conversation: &Arc<Conversation<P>>, state: &UiState,
    cancel_tx: &tokio_mpsc::UnboundedSender<Result<(), ConversationError>>,
    cancel_started: &mut bool, cancel_deadline: &mut Option<tokio::time::Instant>,
    activity: &Arc<Activity>, input: &mut I,
) -> ConsoleResult<()> {
    if *cancel_started { return Ok(()); }
    let Some(session_id) = state.session_id.clone() else { return Ok(()); };
    *cancel_started = true;
    *cancel_deadline = Some(tokio::time::Instant::now() + Duration::from_secs(5));
    activity.stop()?;
    input.update_prompt(current_prompt(state))?;
    let conversation = conversation.clone();
    let cancel_tx = cancel_tx.clone();
    tokio::spawn(async move { let _ = cancel_tx.send(conversation.cancel(&session_id).await); });
    Ok(())
}

async fn wait_until(deadline: Option<tokio::time::Instant>) {
    if let Some(deadline) = deadline {
        tokio::time::sleep_until(deadline).await;
    } else {
        std::future::pending::<()>().await;
    }
}

async fn dispatch_event<I: ConsoleInput + ?Sized>(
    event: UiEvent, input: &mut I, output: &Arc<dyn ConsoleOutput>,
    activity: &Arc<Activity>, policy: &Arc<Mutex<PermissionPolicy>>,
    state: &mut UiState, channels: &mut UiChannels,
) -> ConsoleResult<bool> {
    match event {
        UiEvent::Metadata { event, ack } => {
            match event {
                ConversationEvent::ConfigOptions(options) => state.config_options = options,
                ConversationEvent::Usage(usage) => state.usage = Some(usage),
                _ => {},
            }
            let _ = ack.send(Ok(()));
            Ok(false)
        }
        UiEvent::Output { text, markdown, ack } => {
            input.update_prompt(current_prompt(state))?;
            let result = if markdown {
                output.write_markdown(text).await
            } else {
                match output.flush_response().await {
                    Ok(()) => output.write_text(text).await,
                    Err(error) => Err(error),
                }
            };
            let _ = ack.send(result.clone());
            result?;
            Ok(false)
        }
        UiEvent::Permission { request, reply } => {
            output.flush_response().await?;
            resolve_permission(request, reply, input, output, activity, policy, state, channels).await
        }
    }
}

async fn resolve_permission<I: ConsoleInput + ?Sized>(
    request: PermissionRequest, reply: mpsc::SyncSender<PermissionDecision>,
    input: &mut I, output: &Arc<dyn ConsoleOutput>, activity: &Arc<Activity>,
    policy: &Arc<Mutex<PermissionPolicy>>, state: &mut UiState, channels: &mut UiChannels,
) -> ConsoleResult<bool> {
    loop {
        let current = *policy.lock().map_err(|_| "permission policy unavailable")?;
        if let Some(decision) = current.decision(&request) {
            output.write_text(format!("\n{}\n", permission_action(&request, &decision))).await?;
            let _ = reply.send(decision);
            input.update_prompt(current_prompt(state))?;
            return Ok(false);
        }
        input.update_prompt("Permiso (n/0) > ")?;
        let mut text = format!("\nPermiso solicitado: {}\n", request.title);
        for (index, option) in request.options.iter().enumerate() {
            let label = match option.kind.as_str() {
                "allow_once" => "Permitir esta vez", "allow_always" => "Permitir siempre",
                "reject_once" => "Denegar esta vez", "reject_always" => "Denegar siempre",
                _ => option.name.as_str(),
            };
            text.push_str(&format!("{}. {label}\n", index + 1));
        }
        text.push_str("0. Denegar / cancelar\n");
        output.write_text(text).await?;
        let event = tokio::select! {
            input_event = input.next_event() => input_event?,
            signal = tokio::signal::ctrl_c() => {
                signal.map_err(|error| error.to_string())?;
                state.exit_requested = true;
                state.cancel_requested = true;
                let _ = reply.send(PermissionDecision::Cancelled);
                return Ok(true);
            }
        };
        match event {
            InputEvent::Line(line) => {
                let value = line.trim();
                if value.eq_ignore_ascii_case("/permissions auto") {
                    *policy.lock().map_err(|_| "permission policy unavailable")? = PermissionPolicy::AutoApprove;
                    continue;
                }
                if value.eq_ignore_ascii_case("/permissions ask") {
                    *policy.lock().map_err(|_| "permission policy unavailable")? = PermissionPolicy::Ask;
                    continue;
                }
                if value.eq_ignore_ascii_case("/cancel") || value.eq_ignore_ascii_case("/exit") {
                    let exit = value.eq_ignore_ascii_case("/exit");
                    state.exit_requested |= exit;
                    state.cancel_requested = true;
                    let _ = reply.send(PermissionDecision::Cancelled);
                    activity.stop()?;
                    input.update_prompt(current_prompt(state))?;
                    return Ok(true);
                }
                if value.eq_ignore_ascii_case("/config") {
                    output.write_text(config_text(state, current)).await?;
                    continue;
                }
                if value.starts_with("/image") {
                    if let Some(path) = image_path(&line) {
                        enqueue_image(path, state, &channels.image_tx, output).await?;
                        continue;
                    }
                }
                if let Ok(choice) = value.parse::<usize>() {
                    let decision = if choice == 0 {
                        PermissionDecision::Cancelled
                    } else {
                        request.options.get(choice - 1)
                            .map(|option| PermissionDecision::Selected(option.id.clone()))
                            .unwrap_or(PermissionDecision::Cancelled)
                    };
                    let _ = reply.send(decision);
                    input.update_prompt(current_prompt(state))?;
                    return Ok(false);
                }
                if value.starts_with('/') {
                    state.pending_lines.push_back(line);
                } else {
                    queue_prompt(line, state);
                    output.write_text("Permiso denegado; el mensaje quedó en cola.\n".into()).await?;
                }
                let _ = reply.send(PermissionDecision::Cancelled);
                input.update_prompt(current_prompt(state))?;
                return Ok(false);
            }
            InputEvent::Interrupted | InputEvent::Eof => {
                state.exit_requested = true;
                state.cancel_requested = true;
                let _ = reply.send(PermissionDecision::Cancelled);
                activity.stop()?;
                return Ok(true);
            }
        }
    }
}

async fn enqueue_image(
    path: PathBuf, state: &mut UiState, tx: &tokio_mpsc::UnboundedSender<ImageCompletion>,
    output: &Arc<dyn ConsoleOutput>,
) -> ConsoleResult<()> {
    let id = state.next_image_id;
    state.next_image_id = state.next_image_id.wrapping_add(1);
    state.next_image_ids.push(id);
    let tx = tx.clone();
    tokio::spawn(async move {
        let _ = tx.send((id, images::load_image(path).await));
    });
    output.write_text("Cargando imagen para el siguiente mensaje…\n".into()).await
}

async fn finish_image(
    (id, result): ImageCompletion, output: &Arc<dyn ConsoleOutput>, state: &mut UiState,
) -> ConsoleResult<()> {
    let notice = match &result {
        Ok(_) => "Imagen adjunta. Escribe tu mensaje.\n".to_owned(),
        Err(error) => format!("No se pudo cargar la imagen: {error}\n"),
    };
    state.image_results.insert(id, result);
    output.write_text(notice).await
}

fn queue_prompt(text: String, state: &mut UiState) {
    let image_ids = std::mem::take(&mut state.next_image_ids);
    state.pending_prompts.push_back(PendingPrompt { text, image_ids });
}

fn ready_prompt(state: &mut UiState) -> Option<PendingPrompt> {
    let front = state.pending_prompts.front()?;
    if front.image_ids.iter().any(|id| !state.image_results.contains_key(id)) {
        return None;
    }
    state.pending_prompts.pop_front()
}

fn take_images(state: &mut UiState, ids: &[u64]) -> Result<Vec<ImageAttachment>, String> {
    let mut output = Vec::with_capacity(ids.len());
    for id in ids {
        match state.image_results.remove(id) {
            Some(Ok(image)) => output.push(image),
            Some(Err(error)) => return Err(format!("Mensaje no enviado; falló el adjunto: {error}")),
            None => return Err("Adjunto de imagen pendiente no disponible.".into()),
        }
    }
    Ok(output)
}

fn image_path(line: &str) -> Option<PathBuf> {
    let rest = line.trim().strip_prefix("/image")?.trim();
    if rest.is_empty() { return None; }
    let path = if (rest.starts_with('"') && rest.ends_with('"'))
        || (rest.starts_with('\'') && rest.ends_with('\''))
    {
        &rest[1..rest.len().saturating_sub(1)]
    } else { rest };
    if path.is_empty() { None } else { Some(PathBuf::from(path)) }
}

async fn show_permissions(
    output: &Arc<dyn ConsoleOutput>,
    policy: &Arc<Mutex<PermissionPolicy>>,
) -> ConsoleResult<()> {
    let policy = *policy.lock().map_err(|_| "permission policy unavailable")?;
    output.write_text(format!("Permisos: {}. Usa /permissions auto o /permissions ask.\n", policy_name(policy))).await
}

async fn set_permissions(
    output: &Arc<dyn ConsoleOutput>,
    policy: &Arc<Mutex<PermissionPolicy>>,
    value: PermissionPolicy,
) -> ConsoleResult<()> {
    *policy.lock().map_err(|_| "permission policy unavailable")? = value;
    let message = match value {
        PermissionPolicy::Ask => "Los permisos volverán a preguntarse.\n",
        PermissionPolicy::AutoApprove => "Permisos automáticos activados para esta sesión.\n",
    };
    output.write_text(message.into()).await
}

fn policy_name(value: PermissionPolicy) -> &'static str {
    match value { PermissionPolicy::Ask => "ask", PermissionPolicy::AutoApprove => "auto" }
}

fn config_text(state: &UiState, policy: PermissionPolicy) -> String {
    let agent = state.connection.as_ref().map(|info| info.agent_name.as_str()).unwrap_or("Antigravity");
    let session = if state.session_id.is_some() { "activa" } else { "no iniciada" };
    format!("Agente: {agent}\nSesión: {session}\nPermisos: {}\n", policy_name(policy))
}

fn render_activity<I: ConsoleInput + ?Sized>(
    input: &mut I, activity: &Arc<Activity>, state: &UiState,
) -> ConsoleResult<()> {
    if let Some((frame, label)) = activity.next_frame()? {
        input.update_prompt(&format!("[{frame}] {label} | {}", current_prompt(state)))?;
    }
    Ok(())
}

fn current_prompt(state: &UiState) -> &'static str {
    if state.menu_mode { menu::MENU_PROMPT } else { menu::CHAT_PROMPT }
}

fn start_cancel<P: harness_application::ports::conversation::ConversationProvider + 'static>(
    conversation: &Arc<Conversation<P>>,
    state: &UiState,
    tx: &tokio_mpsc::UnboundedSender<Result<(), ConversationError>>,
    started: &mut bool,
    deadline: &mut Option<tokio::time::Instant>,
) {
    if *started { return; }
    let Some(session_id) = state.session_id.clone() else { return; };
    *started = true;
    *deadline = Some(tokio::time::Instant::now() + Duration::from_secs(5));
    let conversation = conversation.clone();
    let tx = tx.clone();
    tokio::spawn(async move { let _ = tx.send(conversation.cancel(&session_id).await); });
}

async fn wait_deadline(deadline: Option<tokio::time::Instant>) {
    if let Some(deadline) = deadline {
        tokio::time::sleep_until(deadline).await;
    } else {
        std::future::pending::<()>().await;
    }
}

pub async fn login<P: harness_application::ports::conversation::ConversationProvider + 'static>(
    conversation: &Arc<Conversation<P>>,
) -> ConsoleResult<()> {
    login_with_options(conversation, ConsoleOptions::default()).await
}

pub async fn login_with_options<P: harness_application::ports::conversation::ConversationProvider + 'static>(
    conversation: &Arc<Conversation<P>>,
    options: ConsoleOptions,
) -> ConsoleResult<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let (mut input, output, terminal): (Box<dyn ConsoleInput>, Arc<dyn ConsoleOutput>, bool) =
        if stdin.is_terminal() && stdout.is_terminal() {
            let (readline, writer) = Readline::new("Método de inicio de sesión > ".to_owned())
                .map_err(|error| error.to_string())?;
            (
                Box::new(TerminalInput(readline)),
                Arc::new(TerminalOutput(tokio::sync::Mutex::new(writer))),
                true,
            )
        } else {
            let (sender, receiver) = tokio_mpsc::unbounded_channel();
            std::thread::spawn(move || input_thread(io::BufReader::new(stdin), sender));
            (
                Box::new(ChannelInput(receiver)),
                Arc::new(PlainOutput(Arc::new(Mutex::new(stdout)))),
                false,
            )
        };
    let (event_tx, event_rx) = tokio_mpsc::unbounded_channel();
    let (image_tx, image_rx) = tokio_mpsc::unbounded_channel();
    let mut channels = UiChannels { events: event_rx, image_tx, image_rx };
    let policy = Arc::new(Mutex::new(options.permissions));
    let activity = Arc::new(Activity::new(terminal && options.animate));
    let observer = Arc::new(ConsoleObserver {
        events: event_tx,
        permissions: policy.clone(),
        activity: activity.clone(),
    });
    let mut state = UiState::default();
    output.write_text("Antigravity · iniciar sesión con ACP\n".into()).await?;

    activity.start("Conectando con Antigravity…")?;
    let connect = {
        let conversation = conversation.clone();
        let observer = observer.clone();
        spawn_operation(async move { conversation.connect(observer.as_ref()).await })
    };
    state.connection = Some(wait_setup(
        connect, &mut *input, &output, &mut channels, &activity, &policy, &mut state,
    ).await?);
    let auth_methods = state.connection.as_ref().map(|info| info.auth_methods.clone()).unwrap_or_default();
    let Some(method) = choose_auth_method(&auth_methods, &mut *input, &output, &mut state).await? else {
        return Ok(());
    };
    activity.start("Iniciando sesión…")?;
    let authenticate = {
        let conversation = conversation.clone();
        let observer = observer.clone();
        let method_id = method.id;
        spawn_operation(async move { conversation.authenticate(&method_id, observer.as_ref()).await })
    };
    state.connection = Some(wait_setup(
        authenticate, &mut *input, &output, &mut channels, &activity, &policy, &mut state,
    ).await?);
    output.write_text("Inicio de sesión completado.\n".into()).await
}

pub async fn login_with_io<P, R, W>(
    conversation: &Conversation<P>, input: R, output: W,
) -> ConsoleResult<()>
where P: harness_application::ports::conversation::ConversationProvider,
      R: BufRead + Send,
      W: Write + Send {
    let input = Arc::new(Mutex::new(input));
    let output = Arc::new(Mutex::new(output));
    let observer = LoginObserver { input: input.clone(), output: output.clone() };
    {
        let mut output = output.lock().map_err(|_| "console output unavailable")?;
        writeln!(output, "Antigravity · iniciar sesión con ACP\n").map_err(|error| error.to_string())?;
        output.flush().map_err(|error| error.to_string())?;
    }
    let connection = conversation.connect(&observer).await.map_err(|error| error.to_string())?;
    let Some(method) = choose_auth_sync(&connection.auth_methods, &input, &output)? else { return Ok(()); };
    conversation.authenticate(&method.id, &observer).await.map_err(|error| error.to_string())?;
    let mut output = output.lock().map_err(|_| "console output unavailable")?;
    writeln!(output, "Inicio de sesión completado.").map_err(|error| error.to_string())?;
    output.flush().map_err(|error| error.to_string())
}

struct LoginObserver<R,W> { input: Arc<Mutex<R>>, output: Arc<Mutex<W>> }

impl<R:BufRead+Send,W:Write+Send> ConversationObserver for LoginObserver<R,W> {
    fn on_event(&self,event:ConversationEvent)->Result<(),String>{
        if let Some(text)=event_text(event){
            let mut out=self.output.lock().map_err(|_|"console output unavailable")?;
            out.write_all(text.as_bytes()).and_then(|_|out.flush()).map_err(|e|e.to_string())?;
        }
        Ok(())
    }
    fn request_permission(&self,request:PermissionRequest)->Result<PermissionDecision,String>{
        let mut out=self.output.lock().map_err(|_|"console output unavailable")?;
        writeln!(out,"Permiso solicitado: {}",request.title).map_err(|e|e.to_string())?;
        for (i,o) in request.options.iter().enumerate(){writeln!(out,"{}. {}",i+1,o.name).map_err(|e|e.to_string())?;}
        write!(out,"0. Denegar / cancelar > ").map_err(|e|e.to_string())?;
        out.flush().map_err(|e|e.to_string())?;
        drop(out);
        let mut line=String::new();
        self.input.lock().map_err(|_|"console input unavailable")?.read_line(&mut line).map_err(|e|e.to_string())?;
        let Ok(n)=line.trim().parse::<usize>() else{return Ok(PermissionDecision::Cancelled)};
        if n==0{return Ok(PermissionDecision::Cancelled)}
        Ok(request.options.get(n-1).map(|o|PermissionDecision::Selected(o.id.clone())).unwrap_or(PermissionDecision::Cancelled))
    }
}

fn choose_auth_sync<R:BufRead,W:Write>(
    methods:&[AuthMethod],input:&Arc<Mutex<R>>,output:&Arc<Mutex<W>>
)->ConsoleResult<Option<AuthMethod>>{
    if methods.is_empty(){return Ok(None)}
    {
        let mut out=output.lock().map_err(|_|"console output unavailable")?;
        writeln!(out,"Métodos disponibles:").map_err(|e|e.to_string())?;
        for(i,m)in methods.iter().enumerate(){writeln!(out,"{}. {}",i+1,m.name).map_err(|e|e.to_string())?;}
        write!(out,"0. Cancelar > ").map_err(|e|e.to_string())?;
        out.flush().map_err(|e|e.to_string())?;
    }
    let mut line=String::new();
    input.lock().map_err(|_|"console input unavailable")?.read_line(&mut line).map_err(|e|e.to_string())?;
    let Ok(n)=line.trim().parse::<usize>()else{return Ok(None)};
    Ok(n.checked_sub(1).and_then(|i|methods.get(i)).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_application::ports::conversation::{ConversationFuture, ConversationProvider};
    use harness_domain::conversation::{ConnectionInfo, PermissionOption};
    use std::{
        path::Path,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Mutex as StdMutex,
        },
    };

    const TEST_TIMEOUT: Duration = Duration::from_secs(3);

    struct RecordingRenderer(Arc<StdMutex<Vec<String>>>);

    impl MessageRenderer for RecordingRenderer {
        fn render(&self, markdown: &str, _: usize) -> String {
            self.0.lock().unwrap().push(markdown.to_owned());
            format!("rendered:{markdown}")
        }
    }

    #[tokio::test]
    async fn fragmented_markdown_is_rendered_once_without_formatting_ui_notices() {
        let bytes = Arc::new(StdMutex::new(Vec::new()));
        let calls = Arc::new(StdMutex::new(Vec::new()));
        let inner: Arc<dyn ConsoleOutput> = Arc::new(PlainOutput(Arc::new(Mutex::new(CaptureWriter(bytes.clone())))));
        let output = RenderingOutput::new(inner, Arc::new(RecordingRenderer(calls.clone())));
        output.write_markdown("**ho".into()).await.unwrap();
        output.write_text("Mensaje en cola (#1).\n".into()).await.unwrap();
        output.write_markdown("la**\n\n| A | B |\n|---|---|\n| 1 | 2 |".into()).await.unwrap();
        assert!(!calls.lock().unwrap().is_empty(), "complete lines must appear before the turn ends");
        output.flush_response().await.unwrap();
        output.flush_response().await.unwrap();
        assert_eq!(calls.lock().unwrap().concat(), "**hola**\n\n| A | B |\n|---|---|\n| 1 | 2 |");
        let rendered = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
        assert_eq!(rendered.matches("**hola**").count(), 1);
        assert_eq!(rendered.matches("| 1 | 2 |").count(), 1);
        assert!(rendered.starts_with("Mensaje en cola (#1).\n"));
    }

    #[tokio::test]
    async fn response_boundaries_flush_partial_code_and_never_duplicate_it() {
        let bytes = Arc::new(StdMutex::new(Vec::new()));
        let calls = Arc::new(StdMutex::new(Vec::new()));
        let inner: Arc<dyn ConsoleOutput> = Arc::new(PlainOutput(Arc::new(Mutex::new(CaptureWriter(bytes.clone())))));
        let output = RenderingOutput::new(inner, Arc::new(RecordingRenderer(calls.clone())));
        output.write_markdown("```rust\nfn main() {}".into()).await.unwrap();
        output.flush_response().await.unwrap();
        output.write_text("Generación cancelada.\n".into()).await.unwrap();
        output.flush_response().await.unwrap();
        assert_eq!(calls.lock().unwrap().len(), 1);
        assert_eq!(String::from_utf8(bytes.lock().unwrap().clone()).unwrap(),
            "rendered:```rust\nfn main() {}\nGeneración cancelada.\n");
    }

    #[tokio::test]
    async fn failed_response_write_retains_markdown_for_one_successful_retry() {
        struct FailOnceOutput {
            attempts: AtomicUsize,
            bytes: Arc<StdMutex<Vec<u8>>>,
        }

        impl ConsoleOutput for FailOnceOutput {
            fn write_text(&self, text: String) -> LocalFuture<'_, ConsoleResult<()>> {
                Box::pin(async move {
                    if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                        return Err("temporary output failure".into());
                    }
                    self.bytes.lock().unwrap().extend_from_slice(text.as_bytes());
                    Ok(())
                })
            }
        }

        let bytes = Arc::new(StdMutex::new(Vec::new()));
        let calls = Arc::new(StdMutex::new(Vec::new()));
        let inner = Arc::new(FailOnceOutput { attempts: AtomicUsize::new(0), bytes: bytes.clone() });
        let output = RenderingOutput::new(inner.clone(), Arc::new(RecordingRenderer(calls.clone())));
        output.write_markdown("**retained**".into()).await.unwrap();
        assert_eq!(output.flush_response().await.unwrap_err(), "temporary output failure");
        assert!(bytes.lock().unwrap().is_empty());
        output.flush_response().await.unwrap();
        output.flush_response().await.unwrap();
        assert_eq!(inner.attempts.load(Ordering::SeqCst), 2);
        assert_eq!(*calls.lock().unwrap(), ["**retained**", "**retained**"]);
        assert_eq!(String::from_utf8(bytes.lock().unwrap().clone()).unwrap(), "rendered:**retained**\n");
    }

    struct Fake {
        prompts: Arc<StdMutex<Vec<String>>>,
        decisions: Arc<StdMutex<Vec<PermissionDecision>>>,
        completed: Arc<AtomicUsize>,
        initialize_calls: Arc<AtomicUsize>,
        authenticate_calls: Arc<AtomicUsize>,
        authenticate_gate: Arc<tokio::sync::Notify>,
        session_calls: AtomicUsize,
        prompt_calls: AtomicUsize,
        fail_first_session: bool,
        ask_permission: bool,
        first_prompt_gate: Option<Arc<tokio::sync::Notify>>,
    }

    impl ConversationProvider for Fake {
        fn initialize<'a>(
            &'a self,
            _: &'a dyn ConversationObserver,
        ) -> ConversationFuture<'a, ConnectionInfo> {
            Box::pin(async move {
                self.initialize_calls.fetch_add(1, Ordering::SeqCst);
                Ok(ConnectionInfo {
                    agent_name: "fake".into(),
                    auth_methods: vec![AuthMethod {
                        id: "google".into(), name: "Google".into(), description: None,
                    }],
                })
            })
        }

        fn authenticate<'a>(
            &'a self,
            _: &'a str,
            _: &'a dyn ConversationObserver,
        ) -> ConversationFuture<'a, ConnectionInfo> {
            Box::pin(async move {
                self.authenticate_calls.fetch_add(1, Ordering::SeqCst);
                self.authenticate_gate.notified().await;
                Ok(ConnectionInfo {
                    agent_name: "fake".into(),
                    auth_methods: vec![AuthMethod {
                        id: "google".into(), name: "Google".into(), description: None,
                    }],
                })
            })
        }

        fn new_session<'a>(
            &'a self,
            _: &'a Path,
            observer: &'a dyn ConversationObserver,
        ) -> ConversationFuture<'a, SessionInfo> {
            Box::pin(async move {
                let call = self.session_calls.fetch_add(1, Ordering::SeqCst);
                if self.fail_first_session && call == 0 {
                    return Err(ConversationError::Other("Internal error".into()));
                }
                observer
                    .on_event(ConversationEvent::Status("Sesión lista".into()))
                    .map_err(ConversationError::Other)?;
                Ok(SessionInfo { id: "s1".into(), config_options: fake_settings() })
            })
        }

        fn set_config_option<'a>(&'a self, _: &'a str, config_id: &'a str, value: &'a str,
            _: &'a dyn ConversationObserver) -> ConversationFuture<'a, Vec<SessionConfigOption>> {
            Box::pin(async move {
                let mut options = fake_settings();
                let option = options.iter_mut().find(|option| option.id == config_id).unwrap();
                assert!(option.options.iter().any(|item| item.value == value));
                option.current_value = value.to_owned();
                Ok(options)
            })
        }

        fn prompt<'a>(
            &'a self,
            _: &'a str,
            text: &'a str,
            observer: &'a dyn ConversationObserver,
        ) -> ConversationFuture<'a, String> {
            Box::pin(async move {
                let turn = self.prompt_calls.fetch_add(1, Ordering::SeqCst);
                self.prompts.lock().unwrap().push(text.to_owned());
                if self.ask_permission && turn == 0 {
                    let decision = observer
                        .request_permission(PermissionRequest {
                            title: "Ejecutar comando?".into(),
                            options: vec![
                                PermissionOption {
                                    id: "reject_once".into(),
                                    name: "Reject once".into(),
                                    kind: "reject_once".into(),
                                },
                                PermissionOption {
                                    id: "allow_always".into(),
                                    name: "Always allow".into(),
                                    kind: "allow_always".into(),
                                },
                                PermissionOption {
                                    id: "allow_once".into(),
                                    name: "Allow once".into(),
                                    kind: "allow_once".into(),
                                },
                            ],
                        })
                        .map_err(ConversationError::Other)?;
                    self.decisions.lock().unwrap().push(decision);
                }
                if turn == 0 {
                    if let Some(gate) = &self.first_prompt_gate {
                        gate.notified().await;
                    }
                }
                observer
                    .on_event(ConversationEvent::Text(format!("Respuesta: {text}")))
                    .map_err(ConversationError::Other)?;
                self.completed.fetch_add(1, Ordering::SeqCst);
                Ok("end_turn".into())
            })
        }

        fn cancel<'a>(&'a self, _: &'a str) -> ConversationFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }

        fn close<'a>(&'a self) -> ConversationFuture<'a, ()> {
            Box::pin(async { Ok(()) })
        }
    }

    fn fake_settings() -> Vec<SessionConfigOption> {
        use harness_domain::conversation::ConfigValue;
        vec![SessionConfigOption {id:"model-choice".into(),name:"Modelo".into(),category:Some("model".into()),current_value:"a".into(),options:vec![
            ConfigValue{value:"a".into(),name:"Modelo A".into()}, ConfigValue{value:"b".into(),name:"Modelo B".into()}]}]
    }

    struct FakeHandles {
        conversation: Arc<Conversation<Fake>>,
        prompts: Arc<StdMutex<Vec<String>>>,
        decisions: Arc<StdMutex<Vec<PermissionDecision>>>,
        completed: Arc<AtomicUsize>,
        initialize_calls: Arc<AtomicUsize>,
        authenticate_calls: Arc<AtomicUsize>,
        authenticate_gate: Arc<tokio::sync::Notify>,
        gate: Option<Arc<tokio::sync::Notify>>,
    }

    fn fake(
        ask_permission: bool,
        fail_first_session: bool,
        gate_first_prompt: bool,
    ) -> FakeHandles {
        let prompts = Arc::new(StdMutex::new(Vec::new()));
        let decisions = Arc::new(StdMutex::new(Vec::new()));
        let completed = Arc::new(AtomicUsize::new(0));
        let initialize_calls = Arc::new(AtomicUsize::new(0));
        let authenticate_calls = Arc::new(AtomicUsize::new(0));
        let authenticate_gate = Arc::new(tokio::sync::Notify::new());
        let gate = gate_first_prompt.then(|| Arc::new(tokio::sync::Notify::new()));
        let provider = Fake {
            prompts: prompts.clone(),
            decisions: decisions.clone(),
            completed: completed.clone(),
            initialize_calls: initialize_calls.clone(),
            authenticate_calls: authenticate_calls.clone(),
            authenticate_gate: authenticate_gate.clone(),
            session_calls: AtomicUsize::new(0),
            prompt_calls: AtomicUsize::new(0),
            fail_first_session,
            ask_permission,
            first_prompt_gate: gate.clone(),
        };
        FakeHandles {
            conversation: Arc::new(Conversation::new(provider)),
            prompts,
            decisions,
            completed,
            initialize_calls,
            authenticate_calls,
            authenticate_gate,
            gate,
        }
    }

    fn controlled_ui(
        conversation: Arc<Conversation<Fake>>,
        receiver: tokio_mpsc::UnboundedReceiver<InputEvent>,
        output: Arc<StdMutex<Vec<u8>>>,
        permissions: PermissionPolicy,
    ) -> impl Future<Output = ConsoleResult<()>> {
        let output: Arc<dyn ConsoleOutput> = Arc::new(PlainOutput(Arc::new(Mutex::new(CaptureWriter(output)))));
        run_loop(
            conversation,
            Path::new(".").to_path_buf(),
            Box::new(ChannelInput(receiver)),
            output,
            ConsoleOptions {
                permissions,
                animate: false,
            },
            false,
        )
    }

    struct CaptureWriter(Arc<StdMutex<Vec<u8>>>);

    impl Write for CaptureWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().write(bytes)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.0.lock().unwrap().flush()
        }
    }

    fn send_line(sender: &tokio_mpsc::UnboundedSender<InputEvent>, line: &str) {
        sender.send(InputEvent::Line(line.into())).unwrap();
    }

    async fn wait_for_output(output: &Arc<StdMutex<Vec<u8>>>, needle: &str) {
        tokio::time::timeout(TEST_TIMEOUT, async {
            loop {
                let contains = String::from_utf8_lossy(&output.lock().unwrap()).contains(needle);
                if contains {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for console output: {needle}"));
    }

    async fn wait_until(mut predicate: impl FnMut() -> bool) {
        tokio::time::timeout(TEST_TIMEOUT, async {
            while !predicate() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("timed out waiting for test condition");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn model_commands_use_announced_options_without_becoming_prompts() {
        let fake = fake(false, false, false);
        let output = Arc::new(StdMutex::new(Vec::new()));
        let (sender, receiver) = tokio_mpsc::unbounded_channel();
        let ui = tokio::time::timeout(TEST_TIMEOUT, controlled_ui(fake.conversation.clone(), receiver, output.clone(), PermissionPolicy::Ask));
        let driver = async {
            wait_for_output(&output, "Sesión lista").await;
            send_line(&sender, "/model");
            wait_for_output(&output, "Modelo B [b]").await;
            send_line(&sender, "/model 2");
            wait_for_output(&output, "actual: b").await;
            send_line(&sender, "/effort");
            wait_for_output(&output, "no anunció opciones para /effort").await;
            send_line(&sender, "/usage");
            wait_for_output(&output, "Uso no disponible").await;
            send_line(&sender, "/exit");
        };
        let (result, driver) = tokio::join!(ui, tokio::time::timeout(TEST_TIMEOUT, driver));
        driver.unwrap();
        result.unwrap().unwrap();
        assert!(fake.prompts.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn permission_choice_is_not_a_prompt_and_queued_text_runs_fifo() {
        let fake = fake(true, false, true);
        let output = Arc::new(StdMutex::new(Vec::new()));
        let (sender, receiver) = tokio_mpsc::unbounded_channel();
        let ui = tokio::time::timeout(
            TEST_TIMEOUT,
            controlled_ui(
                fake.conversation.clone(),
                receiver,
                output.clone(),
                PermissionPolicy::Ask,
            ),
        );
        let driver = async {
            wait_for_output(&output, "Sesión lista").await;
            send_line(&sender, "hola");
            wait_for_output(&output, "Permiso solicitado: Ejecutar comando?").await;
            send_line(&sender, "3");
            wait_until(|| fake.decisions.lock().unwrap().len() == 1).await;
            send_line(&sender, "siguiente");
            wait_for_output(&output, "Mensaje en cola (#1)").await;
            assert_eq!(*fake.prompts.lock().unwrap(), ["hola"]);
            fake.gate.as_ref().unwrap().notify_one();
            wait_until(|| fake.completed.load(Ordering::SeqCst) == 2).await;
            wait_for_output(&output, "Respuesta: siguiente").await;
            send_line(&sender, "/exit");
        };
        let (ui_result, driver_result) = tokio::join!(ui, tokio::time::timeout(TEST_TIMEOUT, driver));

        driver_result.expect("test driver timed out");
        ui_result.expect("console event loop timed out").unwrap();
        assert_eq!(*fake.prompts.lock().unwrap(), ["hola", "siguiente"]);
        assert_eq!(
            *fake.decisions.lock().unwrap(),
            [PermissionDecision::Selected("allow_once".into())]
        );
        assert!(!String::from_utf8_lossy(&output.lock().unwrap()).contains('\u{1b}'));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn auto_approval_uses_allow_once_without_waiting_for_a_choice() {
        let fake = fake(true, false, false);
        let output = Arc::new(StdMutex::new(Vec::new()));
        let (sender, receiver) = tokio_mpsc::unbounded_channel();
        let ui = tokio::time::timeout(
            TEST_TIMEOUT,
            controlled_ui(
                fake.conversation.clone(),
                receiver,
                output.clone(),
                PermissionPolicy::AutoApprove,
            ),
        );
        let driver = async {
            wait_for_output(&output, "Sesión lista").await;
            send_line(&sender, "hola");
            wait_for_output(&output, "Permiso aprobado automáticamente: Allow once.").await;
            wait_until(|| fake.completed.load(Ordering::SeqCst) == 1).await;
            send_line(&sender, "/exit");
        };
        let (ui_result, driver_result) = tokio::join!(ui, tokio::time::timeout(TEST_TIMEOUT, driver));

        driver_result.expect("test driver timed out");
        ui_result.expect("console event loop timed out").unwrap();
        assert_eq!(*fake.prompts.lock().unwrap(), ["hola"]);
        assert_eq!(
            *fake.decisions.lock().unwrap(),
            [PermissionDecision::Selected("allow_once".into())]
        );
        assert!(!String::from_utf8_lossy(&output.lock().unwrap())
            .contains("Permiso solicitado:"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn failed_initial_session_can_be_recovered_with_new_command() {
        let fake = fake(false, true, false);
        let output = Arc::new(StdMutex::new(Vec::new()));
        let (sender, receiver) = tokio_mpsc::unbounded_channel();
        let ui = tokio::time::timeout(
            TEST_TIMEOUT,
            controlled_ui(
                fake.conversation.clone(),
                receiver,
                output.clone(),
                PermissionPolicy::Ask,
            ),
        );
        let driver = async {
            wait_for_output(&output, "No se pudo abrir una sesión nueva: Internal error").await;
            send_line(&sender, "/new");
            wait_for_output(&output, "Nueva sesión iniciada.").await;
            send_line(&sender, "/menu");
            wait_for_output(&output, "Opciones de conversación").await;
            send_line(&sender, "0");
            send_line(&sender, "1");
            wait_until(|| fake.completed.load(Ordering::SeqCst) == 1).await;
            send_line(&sender, "/exit");
        };
        let (ui_result, driver_result) = tokio::join!(ui, tokio::time::timeout(TEST_TIMEOUT, driver));

        driver_result.expect("test driver timed out");
        ui_result.expect("console event loop timed out").unwrap();
        assert_eq!(fake.initialize_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fake.authenticate_calls.load(Ordering::SeqCst), 0);
        assert_eq!(*fake.prompts.lock().unwrap(), ["1"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn login_authenticates_once_and_preserves_numeric_chat_entered_during_auth() {
        let fake = fake(false, false, false);
        let output = Arc::new(StdMutex::new(Vec::new()));
        let (sender, receiver) = tokio_mpsc::unbounded_channel();
        let ui = tokio::time::timeout(
            TEST_TIMEOUT,
            controlled_ui(
                fake.conversation.clone(), receiver, output.clone(), PermissionPolicy::Ask,
            ),
        );
        let driver = async {
            wait_for_output(&output, "Sesión lista").await;
            send_line(&sender, "/login");
            wait_for_output(&output, "1. Google").await;
            send_line(&sender, "1");
            wait_until(|| fake.authenticate_calls.load(Ordering::SeqCst) == 1).await;
            send_line(&sender, "1");
            fake.authenticate_gate.notify_one();
            wait_for_output(&output, "Sesión iniciada.").await;
            wait_until(|| fake.completed.load(Ordering::SeqCst) == 1).await;
            send_line(&sender, "/exit");
        };
        let (ui_result, driver_result) = tokio::join!(ui, tokio::time::timeout(TEST_TIMEOUT, driver));

        driver_result.expect("test driver timed out");
        ui_result.expect("console event loop timed out").unwrap();
        assert_eq!(fake.initialize_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fake.authenticate_calls.load(Ordering::SeqCst), 1);
        assert_eq!(*fake.prompts.lock().unwrap(), ["1"]);
    }
}

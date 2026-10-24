use super::*;
use super::{
    output::RenderingOutput,
    ports::LocalFuture,
    terminal::{ChannelInput, PlainOutput},
};
use crate::rendering::{self, MessageRenderer};
use harness_application::conversation::Conversation;
use harness_application::ports::conversation::{ConversationFuture, ConversationProvider};
use harness_domain::conversation::{
    AuthMethod, ConversationError, ConversationObserver, ImageAttachment, PermissionDecision,
    PermissionRequest, SessionConfigOption, SessionInfo,
};
use harness_domain::conversation::{ConnectionInfo, PermissionOption};
use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
    time::Duration,
};
use std::{
    path::Path,
    sync::{
        Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::mpsc as tokio_mpsc;

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
    let inner: Arc<dyn ConsoleOutput> = Arc::new(PlainOutput(Arc::new(Mutex::new(CaptureWriter(
        bytes.clone(),
    )))));
    let output = RenderingOutput::new(inner, Arc::new(RecordingRenderer(calls.clone())));
    output.write_markdown("**ho".into()).await.unwrap();
    output
        .write_text("Mensaje en cola (#1).\n".into())
        .await
        .unwrap();
    output
        .write_markdown("la**\n\n| A | B |\n|---|---|\n| 1 | 2 |".into())
        .await
        .unwrap();
    assert!(
        !calls.lock().unwrap().is_empty(),
        "complete lines must appear before the turn ends"
    );
    output.flush_response().await.unwrap();
    output.flush_response().await.unwrap();
    assert_eq!(
        calls.lock().unwrap().concat(),
        "**hola**\n\n| A | B |\n|---|---|\n| 1 | 2 |"
    );
    let rendered = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    assert_eq!(rendered.matches("**hola**").count(), 1);
    assert_eq!(rendered.matches("| 1 | 2 |").count(), 1);
    assert!(rendered.starts_with("Mensaje en cola (#1).\n"));
}

#[tokio::test]
async fn response_boundaries_flush_partial_code_and_never_duplicate_it() {
    let bytes = Arc::new(StdMutex::new(Vec::new()));
    let calls = Arc::new(StdMutex::new(Vec::new()));
    let inner: Arc<dyn ConsoleOutput> = Arc::new(PlainOutput(Arc::new(Mutex::new(CaptureWriter(
        bytes.clone(),
    )))));
    let output = RenderingOutput::new(inner, Arc::new(RecordingRenderer(calls.clone())));
    output
        .write_markdown("```rust\nfn main() {}".into())
        .await
        .unwrap();
    output.flush_response().await.unwrap();
    output
        .write_text("Generación cancelada.\n".into())
        .await
        .unwrap();
    output.flush_response().await.unwrap();
    assert_eq!(calls.lock().unwrap().concat(), "```rust\nfn main() {}");
    assert_eq!(
        String::from_utf8(bytes.lock().unwrap().clone()).unwrap(),
        "rendered:```rust\nrendered:fn main() {}\nGeneración cancelada.\n"
    );
}

#[tokio::test]
async fn redirected_markdown_does_not_gain_artificial_newlines() {
    let bytes = Arc::new(StdMutex::new(Vec::new()));
    let inner: Arc<dyn ConsoleOutput> = Arc::new(PlainOutput(Arc::new(Mutex::new(CaptureWriter(
        bytes.clone(),
    )))));
    let output = RenderingOutput::for_terminal(inner, rendering::renderer(false), false);
    let source = "**hola** código λ ".repeat(40);
    output.write_markdown(source.clone()).await.unwrap();
    assert!(
        !bytes.lock().unwrap().is_empty(),
        "long markdown must stream before finish"
    );
    output.flush_response().await.unwrap();
    output.flush_response().await.unwrap();
    assert_eq!(
        String::from_utf8(bytes.lock().unwrap().clone()).unwrap(),
        source
    );
}

#[tokio::test]
async fn clipboard_image_waits_for_preparation_without_losing_the_queued_prompt() {
    let bytes = Arc::new(StdMutex::new(Vec::new()));
    let output: Arc<dyn ConsoleOutput> =
        Arc::new(PlainOutput(Arc::new(Mutex::new(CaptureWriter(bytes)))));
    let (_sender, receiver) = tokio_mpsc::unbounded_channel();
    let mut input = ChannelInput(receiver);
    let mut state = UiState::default();
    state.next_image_ids.push(7);
    queue_prompt("analiza la captura".into(), &mut state);
    assert!(ready_prompt(&mut state).is_none());
    finish_clipboard(
        7,
        Ok(Some(crate::clipboard::ClipboardContent::Image(
            ImageAttachment {
                mime_type: "image/png".into(),
                data: vec![1, 2, 3],
            },
        ))),
        &mut input,
        &output,
        &mut state,
    )
    .await
    .unwrap();
    let prompt = ready_prompt(&mut state).unwrap();
    assert_eq!(prompt.text, "analiza la captura");
    assert_eq!(
        take_images(&mut state, &prompt.image_ids).unwrap()[0].data,
        [1, 2, 3]
    );
    assert!(ready_prompt(&mut state).is_none());
}

#[tokio::test]
async fn unavailable_clipboard_releases_the_reserved_attachment() {
    let bytes = Arc::new(StdMutex::new(Vec::new()));
    let output: Arc<dyn ConsoleOutput> =
        Arc::new(PlainOutput(Arc::new(Mutex::new(CaptureWriter(bytes)))));
    let (_sender, receiver) = tokio_mpsc::unbounded_channel();
    let mut input = ChannelInput(receiver);
    for result in [Ok(None), Err("portapapeles ocupado".to_owned())] {
        let mut state = UiState::default();
        state.next_image_ids.push(8);
        queue_prompt("mensaje".into(), &mut state);
        finish_clipboard(8, result, &mut input, &output, &mut state)
            .await
            .unwrap();
        let prompt = ready_prompt(&mut state).unwrap();
        assert_eq!(prompt.text, "mensaje");
        assert!(prompt.image_ids.is_empty());
        assert!(state.next_image_ids.is_empty());
    }
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
                self.bytes
                    .lock()
                    .unwrap()
                    .extend_from_slice(text.as_bytes());
                Ok(())
            })
        }
    }

    let bytes = Arc::new(StdMutex::new(Vec::new()));
    let calls = Arc::new(StdMutex::new(Vec::new()));
    let inner = Arc::new(FailOnceOutput {
        attempts: AtomicUsize::new(0),
        bytes: bytes.clone(),
    });
    let output = RenderingOutput::new(inner.clone(), Arc::new(RecordingRenderer(calls.clone())));
    output.write_markdown("**retained**".into()).await.unwrap();
    assert_eq!(
        output.flush_response().await.unwrap_err(),
        "temporary output failure"
    );
    assert!(bytes.lock().unwrap().is_empty());
    output.flush_response().await.unwrap();
    output.flush_response().await.unwrap();
    assert_eq!(inner.attempts.load(Ordering::SeqCst), 2);
    assert_eq!(*calls.lock().unwrap(), ["**retained**", "**retained**"]);
    assert_eq!(
        String::from_utf8(bytes.lock().unwrap().clone()).unwrap(),
        "rendered:**retained**\n"
    );
}

struct Fake {
    prompts: Arc<StdMutex<Vec<String>>>,
    decisions: Arc<StdMutex<Vec<PermissionDecision>>>,
    completed: Arc<AtomicUsize>,
    cancel_calls: Arc<AtomicUsize>,
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
                    id: "google".into(),
                    name: "Google".into(),
                    description: None,
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
                    id: "google".into(),
                    name: "Google".into(),
                    description: None,
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
            Ok(SessionInfo {
                id: "s1".into(),
                config_options: fake_settings(),
            })
        })
    }

    fn set_config_option<'a>(
        &'a self,
        _: &'a str,
        config_id: &'a str,
        value: &'a str,
        _: &'a dyn ConversationObserver,
    ) -> ConversationFuture<'a, Vec<SessionConfigOption>> {
        Box::pin(async move {
            let mut options = fake_settings();
            let option = options
                .iter_mut()
                .find(|option| option.id == config_id)
                .unwrap();
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
        Box::pin(async move {
            self.cancel_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }

    fn close<'a>(&'a self) -> ConversationFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

fn fake_settings() -> Vec<SessionConfigOption> {
    use harness_domain::conversation::ConfigValue;
    vec![SessionConfigOption {
        id: "model-choice".into(),
        name: "Modelo".into(),
        category: Some("model".into()),
        current_value: "a".into(),
        options: vec![
            ConfigValue {
                value: "a".into(),
                name: "Modelo A".into(),
            },
            ConfigValue {
                value: "b".into(),
                name: "Modelo B".into(),
            },
        ],
    }]
}

struct FakeHandles {
    conversation: Arc<Conversation<Fake>>,
    prompts: Arc<StdMutex<Vec<String>>>,
    decisions: Arc<StdMutex<Vec<PermissionDecision>>>,
    completed: Arc<AtomicUsize>,
    cancel_calls: Arc<AtomicUsize>,
    initialize_calls: Arc<AtomicUsize>,
    authenticate_calls: Arc<AtomicUsize>,
    authenticate_gate: Arc<tokio::sync::Notify>,
    gate: Option<Arc<tokio::sync::Notify>>,
}

fn fake(ask_permission: bool, fail_first_session: bool, gate_first_prompt: bool) -> FakeHandles {
    let prompts = Arc::new(StdMutex::new(Vec::new()));
    let decisions = Arc::new(StdMutex::new(Vec::new()));
    let completed = Arc::new(AtomicUsize::new(0));
    let cancel_calls = Arc::new(AtomicUsize::new(0));
    let initialize_calls = Arc::new(AtomicUsize::new(0));
    let authenticate_calls = Arc::new(AtomicUsize::new(0));
    let authenticate_gate = Arc::new(tokio::sync::Notify::new());
    let gate = gate_first_prompt.then(|| Arc::new(tokio::sync::Notify::new()));
    let provider = Fake {
        prompts: prompts.clone(),
        decisions: decisions.clone(),
        completed: completed.clone(),
        cancel_calls: cancel_calls.clone(),
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
        cancel_calls,
        initialize_calls,
        authenticate_calls,
        authenticate_gate,
        gate,
    }
}

struct EmptyClipboard;

impl super::ports::ClipboardReader for EmptyClipboard {
    fn read(&self) -> super::ports::ClipboardFuture<'_> {
        Box::pin(async { Ok(None) })
    }
}

fn controlled_ui(
    conversation: Arc<Conversation<Fake>>,
    receiver: tokio_mpsc::UnboundedReceiver<InputEvent>,
    output: Arc<StdMutex<Vec<u8>>>,
    permissions: PermissionPolicy,
) -> impl Future<Output = ConsoleResult<()>> {
    let output: Arc<dyn ConsoleOutput> =
        Arc::new(PlainOutput(Arc::new(Mutex::new(CaptureWriter(output)))));
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
        Arc::new(EmptyClipboard),
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

fn send_interrupt(sender: &tokio_mpsc::UnboundedSender<InputEvent>) {
    sender.send(InputEvent::Interrupted).unwrap();
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
        send_line(&sender, "/model");
        wait_for_output(&output, "Modelo B [b]").await;
        send_line(&sender, "2");
        wait_for_output(&output, "Modelo actualizado.").await;
        output.lock().unwrap().clear();
        send_line(&sender, "/model 1");
        wait_for_output(&output, "Modelo actualizado.").await;
        output.lock().unwrap().clear();
        send_line(&sender, "/model");
        wait_for_output(&output, "Modelo B [b]").await;
        send_line(&sender, "no válido");
        wait_for_output(&output, "Opción inválida").await;
        send_line(&sender, "0");
        wait_for_output(&output, "Sin cambios.").await;
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
        send_line(&sender, "/model");
        wait_for_output(
            &output,
            "Cambio de configuración en cola para después del turno.",
        )
        .await;
        fake.gate.as_ref().unwrap().notify_one();
        wait_for_output(&output, "Modelo B [b]").await;
        send_line(&sender, "2");
        wait_for_output(&output, "Modelo actualizado.").await;
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
    assert!(!String::from_utf8_lossy(&output.lock().unwrap()).contains("Permiso solicitado:"));
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
            fake.conversation.clone(),
            receiver,
            output.clone(),
            PermissionPolicy::Ask,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_interrupt_cancels_generation_and_chat_continues_without_stale_exit() {
    let fake = fake(false, false, true);
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
        send_line(&sender, "primero");
        wait_until(|| fake.prompts.lock().unwrap().len() == 1).await;
        send_interrupt(&sender);
        wait_until(|| fake.cancel_calls.load(Ordering::SeqCst) == 1).await;
        fake.gate.as_ref().unwrap().notify_one();
        wait_for_output(&output, "Generación cancelada.").await;

        send_line(&sender, "segundo");
        wait_until(|| fake.completed.load(Ordering::SeqCst) == 2).await;
        wait_for_output(&output, "Respuesta: segundo").await;
        send_line(&sender, "/exit");
    };
    let (ui_result, driver_result) = tokio::join!(ui, tokio::time::timeout(TEST_TIMEOUT, driver));

    driver_result.expect("test driver timed out");
    ui_result.expect("console event loop timed out").unwrap();
    assert_eq!(*fake.prompts.lock().unwrap(), ["primero", "segundo"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn second_interrupt_during_cancellation_exits_immediately() {
    let fake = fake(false, false, true);
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
        send_line(&sender, "detener");
        wait_until(|| fake.prompts.lock().unwrap().len() == 1).await;
        send_interrupt(&sender);
        wait_until(|| fake.cancel_calls.load(Ordering::SeqCst) == 1).await;
        send_interrupt(&sender);
    };
    let (ui_result, driver_result) = tokio::join!(ui, tokio::time::timeout(TEST_TIMEOUT, driver));

    driver_result.expect("test driver timed out");
    ui_result
        .expect("console did not exit on second interrupt")
        .unwrap();
    assert_eq!(*fake.prompts.lock().unwrap(), ["detener"]);
    assert_eq!(fake.completed.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interrupt_arm_survives_cancel_completion_until_idle_second_press() {
    let fake = fake(false, false, true);
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
        send_line(&sender, "cancelar");
        wait_until(|| fake.prompts.lock().unwrap().len() == 1).await;
        send_interrupt(&sender);
        wait_until(|| fake.cancel_calls.load(Ordering::SeqCst) == 1).await;
        fake.gate.as_ref().unwrap().notify_one();
        wait_for_output(&output, "Generación cancelada.").await;
        send_interrupt(&sender);
    };
    let (ui_result, driver_result) = tokio::join!(ui, tokio::time::timeout(TEST_TIMEOUT, driver));

    driver_result.expect("test driver timed out");
    ui_result
        .expect("idle second interrupt did not exit after cancellation")
        .unwrap();
    assert_eq!(*fake.prompts.lock().unwrap(), ["cancelar"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_interrupt_arms_exit_and_new_prompt_clears_the_arm() {
    let fake = fake(false, false, false);
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
        // The session status is emitted before setup completes; establish
        // that the idle loop is actually running before testing interrupts.
        send_line(&sender, "/model 1");
        wait_for_output(&output, "Modelo actualizado.").await;
        output.lock().unwrap().clear();
        send_interrupt(&sender);
        wait_for_output(&output, "Presiona Ctrl+C otra vez para salir.").await;

        send_line(&sender, "seguir");
        wait_for_output(&output, "Respuesta: seguir").await;
        wait_until(|| fake.completed.load(Ordering::SeqCst) == 1).await;

        // A provider can finish before the UI consumes its completion. Use
        // a configuration command as an idle-loop barrier, not a sleep.
        send_line(&sender, "/model 1");
        wait_for_output(&output, "Modelo actualizado.").await;

        send_interrupt(&sender);
        wait_until(|| {
            String::from_utf8_lossy(&output.lock().unwrap())
                .matches("Presiona Ctrl+C otra vez para salir.")
                .count()
                >= 2
        })
        .await;
        send_interrupt(&sender);
    };
    let (ui_result, driver_result) = tokio::join!(ui, tokio::time::timeout(TEST_TIMEOUT, driver));

    driver_result.expect("test driver timed out");
    ui_result
        .expect("console did not exit on second idle interrupt")
        .unwrap();
    assert_eq!(*fake.prompts.lock().unwrap(), ["seguir"]);
}

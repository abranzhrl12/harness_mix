//! Abortable provider work and responsive setup waiting.
use super::{ConsoleInput, ConsoleOutput, ConsoleResult, InputEvent, UiChannels, UiState};
use super::{current_prompt, dispatch_event, finish_image, render_activity, request_clipboard};
use crate::terminal::activity::Activity;
use futures_util::future::{AbortHandle, Abortable};
use harness_domain::conversation::{ConversationError, PermissionPolicy};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::task::JoinHandle;

pub(super) struct OperationHandle<T> {
    task: JoinHandle<Result<Result<T, ConversationError>, futures_util::future::Aborted>>,
    abort: AbortHandle,
}

impl<T> OperationHandle<T> {
    pub(super) fn abort(&self) {
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
            std::task::Poll::Ready(Err(error)) => std::task::Poll::Ready(Err(error.to_string())),
        }
    }
}

pub(super) fn spawn_operation<T, F>(future: F) -> OperationHandle<T>
where
    T: Send + 'static,
    F: Future<Output = Result<T, ConversationError>> + Send + 'static,
{
    let runtime = tokio::runtime::Handle::current();
    let (abort, registration) = AbortHandle::new_pair();
    let task =
        tokio::task::spawn_blocking(move || runtime.block_on(Abortable::new(future, registration)));
    OperationHandle { task, abort }
}

pub(super) async fn wait_setup<T, I>(
    mut task: OperationHandle<T>,
    input: &mut I,
    output: &Arc<dyn ConsoleOutput>,
    channels: &mut UiChannels,
    activity: &Arc<Activity>,
    policy: &Arc<Mutex<PermissionPolicy>>,
    state: &mut UiState,
) -> ConsoleResult<T>
where
    I: ConsoleInput + ?Sized,
    T: Send + 'static,
{
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
                InputEvent::PasteRequested => request_clipboard(state, channels)?,
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

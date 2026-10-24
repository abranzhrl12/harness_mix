//! Conversation orchestration over injected input/output ports.
use super::{ConsoleInput, ConsoleOptions, ConsoleOutput, ConsoleResult, InputEvent};
use super::{context::UiRuntime, output::RenderingOutput, ports::ClipboardReader};
use super::{
    current_prompt, dispatch_event, finish_image, handle_live_line, interrupt_exits_when_armed,
    is_setting_command, open_session, process_idle_line, ready_prompt, render_activity,
    request_clipboard, spawn_operation, start_cancel, take_images, wait_setup, wait_until,
};
use crate::{menu, rendering};
use harness_application::conversation::Conversation;
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::mpsc as tokio_mpsc;

pub(super) async fn run_loop<P>(
    conversation: Arc<Conversation<P>>,
    workspace: PathBuf,
    mut input: Box<dyn ConsoleInput>,
    output: Arc<dyn ConsoleOutput>,
    options: ConsoleOptions,
    terminal: bool,
    clipboard: Arc<dyn ClipboardReader>,
) -> ConsoleResult<()>
where
    P: harness_application::ports::conversation::ConversationProvider + 'static,
{
    let styled = terminal
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").map_or(true, |value| value != "dumb");
    let output: Arc<dyn ConsoleOutput> = Arc::new(RenderingOutput::for_terminal(
        output,
        rendering::renderer(styled),
        terminal,
    ));
    let UiRuntime {
        mut channels,
        policy,
        activity,
        observer,
        mut state,
    } = UiRuntime::new(options, terminal, clipboard);

    output
        .write_text(format!("{}\n", menu::render_startup_banner()))
        .await?;
    input.update_prompt(menu::CHAT_PROMPT)?;
    input.set_clipboard_available(true)?;

    activity.start("Conectando con Antigravity…")?;
    let connect = spawn_operation({
        let conversation = conversation.clone();
        let observer = observer.clone();
        async move { conversation.connect(observer.as_ref()).await }
    });
    state.connection = Some(
        wait_setup(
            connect,
            &mut *input,
            &output,
            &mut channels,
            &activity,
            &policy,
            &mut state,
        )
        .await?,
    );

    state.session_id = open_session(
        &conversation,
        &workspace,
        &observer,
        &mut *input,
        &output,
        &mut channels,
        &activity,
        &policy,
        &mut state,
    )
    .await?;

    let mut interval = tokio::time::interval(Duration::from_millis(100));
    loop {
        if state.exit_requested {
            break;
        }

        // Apply a model/effort change queued during the previous turn before
        // dispatching the next queued user prompt.
        if state
            .pending_lines
            .front()
            .is_some_and(|line| is_setting_command(line))
        {
            let line = state.pending_lines.pop_front().unwrap();
            process_idle_line(
                &line,
                &conversation,
                &workspace,
                &observer,
                &mut *input,
                &output,
                &mut channels,
                &activity,
                &policy,
                &mut state,
            )
            .await?;
            continue;
        }

        if let Some(prompt) = ready_prompt(&mut state) {
            state.idle_interrupt_armed = false;
            if state.session_id.is_none() {
                output
                    .write_text("No hay una sesión activa. Usa /new o /login.\n".into())
                    .await?;
                continue;
            }
            let images = match take_images(&mut state, &prompt.image_ids) {
                Ok(images) => images,
                Err(error) => {
                    output.write_text(format!("{error}\n")).await?;
                    continue;
                }
            };
            let session_id = state.session_id.clone().unwrap();
            activity.start("Pensando y escribiendo…")?;
            let mut send_task = spawn_operation({
                let conversation = conversation.clone();
                let observer = observer.clone();
                async move {
                    conversation
                        .send_with_images(&session_id, &prompt.text, &images, observer.as_ref())
                        .await
                }
            });
            let (cancel_tx, mut cancel_rx) = tokio_mpsc::unbounded_channel();
            let mut cancel_started = false;
            let mut interrupt_exit_armed = false;
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
                            InputEvent::Line(line) => {
                                interrupt_exit_armed = false;
                                handle_live_line(
                                    &line, &conversation, &mut *input, &output, &activity, &policy,
                                    &mut state, &mut channels, &cancel_tx, &mut cancel_started,
                                    &mut cancel_deadline,
                                ).await?
                            }
                            InputEvent::PasteRequested => request_clipboard(&mut state, &channels)?,
                            InputEvent::Interrupted => {
                                if interrupt_exits_when_armed(&mut interrupt_exit_armed) {
                                    state.exit_requested = true;
                                    send_task.abort();
                                    let _ = send_task.await;
                                    activity.stop()?;
                                    output.flush_response().await?;
                                    break;
                                } else if !cancel_started {
                                    start_cancel(&conversation, &state, &cancel_tx, &mut cancel_started, &mut cancel_deadline);
                                }
                            }
                            InputEvent::Eof => {
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
                    signal = tokio::signal::ctrl_c(), if !terminal => {
                        signal.map_err(|error| error.to_string())?;
                        if interrupt_exits_when_armed(&mut interrupt_exit_armed) {
                            state.exit_requested = true;
                            send_task.abort();
                            let _ = send_task.await;
                            activity.stop()?;
                            output.flush_response().await?;
                            break;
                        } else if !cancel_started {
                            start_cancel(&conversation, &state, &cancel_tx, &mut cancel_started, &mut cancel_deadline);
                        }
                    }
                }
            }
            if interrupt_exit_armed {
                state.idle_interrupt_armed = true;
            }
            continue;
        }

        if let Some(line) = state.pending_lines.pop_front() {
            state.idle_interrupt_armed = false;
            process_idle_line(
                &line,
                &conversation,
                &workspace,
                &observer,
                &mut *input,
                &output,
                &mut channels,
                &activity,
                &policy,
                &mut state,
            )
            .await?;
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
                    InputEvent::Line(line) => {
                        state.idle_interrupt_armed = false;
                        process_idle_line(
                            &line, &conversation, &workspace, &observer, &mut *input, &output,
                            &mut channels, &activity, &policy, &mut state,
                        ).await?
                    }
                    InputEvent::PasteRequested => request_clipboard(&mut state, &channels)?,
                    InputEvent::Interrupted => {
                        state.exit_requested = interrupt_exits_when_armed(&mut state.idle_interrupt_armed);
                        if !state.exit_requested {
                            output.write_text("Presiona Ctrl+C otra vez para salir.\n".into()).await?;
                        }
                    }
                    InputEvent::Eof => state.exit_requested = true,
                }
            }
            image = channels.image_rx.recv() => {
                if let Some(image) = image { finish_image(image, &output, &mut state).await?; }
            }
            _ = interval.tick() => render_activity(&mut *input, &activity, &state)?,
            signal = tokio::signal::ctrl_c(), if !terminal => {
                signal.map_err(|error| error.to_string())?;
                state.exit_requested = interrupt_exits_when_armed(&mut state.idle_interrupt_armed);
                if !state.exit_requested {
                    output.write_text("Presiona Ctrl+C otra vez para salir.\n".into()).await?;
                }
            }
        }
    }

    activity.stop()?;
    output.flush_response().await?;
    conversation
        .close()
        .await
        .map_err(|error| error.to_string())
}

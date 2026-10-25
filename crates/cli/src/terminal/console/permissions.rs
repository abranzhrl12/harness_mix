use harness_domain::conversation::{PermissionDecision, PermissionPolicy, PermissionRequest};
use std::sync::{Arc, Mutex, mpsc};

use super::{
    Activity, ConsoleInput, ConsoleOutput, ConsoleResult, InputEvent, UiChannels, UiEvent, UiState,
    enqueue_image, image_path, permission_action, queue_prompt,
};

pub(super) async fn dispatch_event<I: ConsoleInput + ?Sized>(
    event: UiEvent,
    input: &mut I,
    output: &Arc<dyn ConsoleOutput>,
    activity: &Arc<Activity>,
    policy: &Arc<Mutex<PermissionPolicy>>,
    state: &mut UiState,
    channels: &mut UiChannels,
) -> ConsoleResult<bool> {
    match event {
        UiEvent::Metadata { event, ack } => {
            match event {
                super::ConversationEvent::ConfigOptions(options) => state.config_options = options,
                super::ConversationEvent::Usage(usage) => state.usage = Some(usage),
                _ => {}
            }
            let _ = ack.send(Ok(()));
            Ok(false)
        }
        UiEvent::Output {
            text,
            markdown,
            ack,
        } => {
            input.update_prompt(super::current_prompt(state))?;
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
            input.set_clipboard_available(false)?;
            let result = resolve_permission(
                request, reply, input, output, activity, policy, state, channels,
            )
            .await;
            input.set_clipboard_available(true)?;
            result
        }
        UiEvent::Clipboard { id, result } => {
            super::finish_clipboard(id, result, input, output, state).await?;
            Ok(false)
        }
    }
}

async fn resolve_permission<I: ConsoleInput + ?Sized>(
    request: PermissionRequest,
    reply: mpsc::SyncSender<PermissionDecision>,
    input: &mut I,
    output: &Arc<dyn ConsoleOutput>,
    activity: &Arc<Activity>,
    policy: &Arc<Mutex<PermissionPolicy>>,
    state: &mut UiState,
    channels: &mut UiChannels,
) -> ConsoleResult<bool> {
    loop {
        let current = *policy.lock().map_err(|_| "permission policy unavailable")?;
        if let Some(decision) = current.decision(&request) {
            output
                .write_text(format!("\n{}\n", permission_action(&request, &decision)))
                .await?;
            let _ = reply.send(decision);
            input.update_prompt(super::current_prompt(state))?;
            return Ok(false);
        }
        input.update_prompt("Permiso (n/0) > ")?;
        let mut text = format!("\nPermiso solicitado: {}\n", request.title);
        for (index, option) in request.options.iter().enumerate() {
            let label = match option.kind.as_str() {
                "allow_once" => "Permitir esta vez",
                "allow_always" => "Permitir siempre",
                "reject_once" => "Denegar esta vez",
                "reject_always" => "Denegar siempre",
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
                    *policy.lock().map_err(|_| "permission policy unavailable")? =
                        PermissionPolicy::AutoApprove;
                    continue;
                }
                if value.eq_ignore_ascii_case("/permissions ask") {
                    *policy.lock().map_err(|_| "permission policy unavailable")? =
                        PermissionPolicy::Ask;
                    continue;
                }
                if value.eq_ignore_ascii_case("/cancel") || value.eq_ignore_ascii_case("/exit") {
                    let exit = value.eq_ignore_ascii_case("/exit");
                    state.exit_requested |= exit;
                    state.cancel_requested = true;
                    let _ = reply.send(PermissionDecision::Cancelled);
                    activity.stop()?;
                    input.update_prompt(super::current_prompt(state))?;
                    return Ok(true);
                }
                if value.eq_ignore_ascii_case("/config") {
                    output
                        .write_text(super::configuration::config_text(state, current))
                        .await?;
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
                        request
                            .options
                            .get(choice - 1)
                            .map(|option| PermissionDecision::Selected(option.id.clone()))
                            .unwrap_or(PermissionDecision::Cancelled)
                    };
                    let _ = reply.send(decision);
                    input.update_prompt(super::current_prompt(state))?;
                    return Ok(false);
                }
                if value.starts_with('/') {
                    state.pending_lines.push_back(line);
                } else {
                    queue_prompt(line, state);
                    output
                        .write_text("Permiso denegado; el mensaje quedó en cola.\n".into())
                        .await?;
                }
                let _ = reply.send(PermissionDecision::Cancelled);
                input.update_prompt(super::current_prompt(state))?;
                return Ok(false);
            }
            InputEvent::PasteRequested => continue,
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

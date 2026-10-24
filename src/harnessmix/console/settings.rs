use crate::{rendering, session_settings};
use harness_application::conversation::Conversation;
use harness_domain::conversation::PermissionPolicy;
use std::sync::{Arc, Mutex};

use super::{Activity, ConsoleInput, ConsoleObserver, ConsoleOutput, spawn_operation, wait_setup};
use super::{ConsoleResult, InputEvent, UiChannels, UiState};

pub(super) async fn change_session_setting<P, I>(
    line: &str,
    conversation: &Arc<Conversation<P>>,
    observer: &Arc<ConsoleObserver>,
    input: &mut I,
    output: &Arc<dyn ConsoleOutput>,
    channels: &mut UiChannels,
    activity: &Arc<Activity>,
    policy: &Arc<Mutex<PermissionPolicy>>,
    state: &mut UiState,
) -> ConsoleResult<()>
where
    P: harness_application::ports::conversation::ConversationProvider + 'static,
    I: ConsoleInput + ?Sized,
{
    let mut words = line.split_whitespace();
    let command = words.next().unwrap_or("/model");
    let value = words.next();
    if words.next().is_some() {
        return output
            .write_text(format!("Uso: {command} <número o id>.\n"))
            .await;
    }
    let Some(option) = session_settings::selector(&state.config_options, command).cloned() else {
        return output
            .write_text(session_settings::list(&state.config_options, command))
            .await;
    };
    let selected = if let Some(value) = value {
        match session_settings::selected_value(&option, value) {
            Ok(value) => value.to_owned(),
            Err(message) => return output.write_text(message).await,
        }
    } else {
        if option.options.is_empty() {
            return output
                .write_text(session_settings::list(&state.config_options, command))
                .await;
        }
        let choices = option
            .options
            .iter()
            .enumerate()
            .map(|(index, choice)| {
                format!(
                    "{}. {}{}",
                    index + 1,
                    rendering::sanitize(&choice.name),
                    if choice.value == option.current_value {
                        " (actual)"
                    } else {
                        ""
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(" | ");
        let prompt = format!(
            "{}: {} | 0 cancelar > ",
            rendering::sanitize(&option.name),
            choices
        );
        let choice: ConsoleResult<Option<String>> = async {
            let transient = input.set_selector_mode(true, &prompt)?;
            if !transient {
                output
                    .write_text(session_settings::list(&state.config_options, command))
                    .await?;
            }
            loop {
                let event = match state.pending_lines.pop_front() {
                    Some(line) => InputEvent::Line(line),
                    None => input.next_event().await?,
                };
                match event {
                    InputEvent::Line(choice) if choice.trim() == "0" => return Ok(None),
                    InputEvent::Line(choice)
                        if matches!(choice.trim(), "/exit" | "/quit") =>
                    {
                        state.exit_requested = true;
                        return Ok(None);
                    }
                    InputEvent::Line(choice) => {
                        match session_settings::selected_value(&option, choice.trim()) {
                            Ok(value) => return Ok(Some(value.to_owned())),
                            Err(_) => {
                                output
                                    .write_text(
                                        "Opción inválida; intenta otra vez o escribe 0 para cancelar.\n"
                                            .into(),
                                    )
                                    .await?;
                            }
                        }
                    }
                    // Clipboard interception is disabled while a selector owns input.
                    InputEvent::PasteRequested => {}
                    InputEvent::Eof | InputEvent::Interrupted => return Ok(None),
                }
            }
        }
        .await;
        let restored = input.set_selector_mode(false, super::current_prompt(state));
        let choice = choice?;
        restored?;
        let Some(choice) = choice else {
            return output.write_text("Sin cambios.\n".into()).await;
        };
        choice
    };
    let config_id = option.id.clone();
    let setting_name = option.name.clone();
    let confirmation_id = config_id.clone();
    let Some(session_id) = state.session_id.clone() else {
        return output
            .write_text("No hay una sesión activa. Usa /new.\n".into())
            .await;
    };
    let expected_value = selected.clone();
    activity.start("Actualizando configuración…")?;
    let task = spawn_operation({
        let conversation = conversation.clone();
        let observer = observer.clone();
        async move {
            conversation
                .set_config_option(&session_id, &config_id, &selected, observer.as_ref())
                .await
        }
    });
    match wait_setup(task, input, output, channels, activity, policy, state).await {
        Ok(options) => {
            state.config_options = options;
            let actual_value = state
                .config_options
                .iter()
                .find(|option| option.id == confirmation_id)
                .map(|option| option.current_value.as_str());
            if actual_value == Some(expected_value.as_str()) {
                output
                    .write_text(format!("{setting_name} actualizado.\n"))
                    .await
            } else {
                output
                    .write_text(format!(
                        "El servidor no confirmó el cambio de {setting_name}; valor actual: {}.\n",
                        actual_value.unwrap_or("no disponible")
                    ))
                    .await
            }
        }
        Err(message) => {
            output
                .write_text(format!("No se pudo cambiar la configuración: {message}\n"))
                .await
        }
    }
}

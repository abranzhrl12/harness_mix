use crate::{menu, session_settings};
use harness_application::conversation::Conversation;
use harness_domain::conversation::PermissionPolicy;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use super::{
    Activity, ConsoleInput, ConsoleObserver, ConsoleOutput, ConsoleResult, UiChannels, UiState,
    enqueue_image, image_path, queue_prompt,
};
use super::{
    configuration::{config_text, set_permissions, show_permissions},
    session::{open_session, perform_login},
    settings::change_session_setting,
};

pub(super) async fn process_idle_line<P, I>(
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
where
    P: harness_application::ports::conversation::ConversationProvider + 'static,
    I: ConsoleInput + ?Sized,
{
    if state.menu_mode {
        return handle_menu_choice(
            line,
            false,
            conversation,
            workspace,
            observer,
            input,
            output,
            channels,
            activity,
            policy,
            state,
        )
        .await;
    }
    let trimmed = line.trim();
    match trimmed {
        "" => Ok(()),
        "/menu" => {
            state.menu_mode = true;
            output.write_text(menu::render_menu().into()).await?;
            input.update_prompt(menu::MENU_PROMPT)
        }
        "/exit" | "/quit" => {
            state.exit_requested = true;
            Ok(())
        }
        "/config" => {
            output
                .write_text(config_text(
                    state,
                    *policy
                        .lock()
                        .map_err(|_| "permission policy unavailable")?,
                ))
                .await
        }
        "/usage" => output.write_text(session_settings::usage(state.usage.as_ref())).await,
        _ if is_setting_command(trimmed) => {
            change_session_setting(
                trimmed, conversation, observer, input, output, channels, activity, policy, state,
            )
            .await
        }
        "/permissions" => show_permissions(output, policy).await,
        "/permissions auto" => set_permissions(output, policy, PermissionPolicy::AutoApprove).await,
        "/permissions ask" => set_permissions(output, policy, PermissionPolicy::Ask).await,
        "/cancel" => {
            output
                .write_text("No hay una respuesta en curso para cancelar.\n".into())
                .await
        }
        "/new" => {
            state.session_id = open_session(
                conversation, workspace, observer, input, output, channels, activity, policy, state,
            )
            .await?;
            if state.session_id.is_some() {
                output.write_text("Nueva sesión iniciada.\n".into()).await?;
            }
            Ok(())
        }
        "/login" => {
            perform_login(
                conversation, workspace, observer, input, output, channels, activity, policy, state,
            )
            .await
        }
        _ if trimmed.starts_with("/image") => {
            if let Some(path) = image_path(line) {
                enqueue_image(path, state, &channels.image_tx, output).await
            } else {
                output
                    .write_text("Uso: /image <ruta> (entre comillas si contiene espacios).\n".into())
                    .await
            }
        }
        _ if trimmed.starts_with('/') => output
            .write_text("Comandos: /menu, /login, /new, /model [opción], /effort [opción], /usage, /config, /permissions [auto|ask], /image <ruta>, /exit\n".into())
            .await,
        _ if state.session_id.is_none() => output
            .write_text("No hay una sesión activa. Usa /new o /login.\n".into())
            .await,
        _ => {
            queue_prompt(line.to_owned(), state);
            Ok(())
        }
    }
}

pub(super) fn is_setting_command(line: &str) -> bool {
    matches!(line.split_whitespace().next(), Some("/model" | "/effort"))
}

pub(super) async fn handle_menu_choice<P, I>(
    line: &str,
    active: bool,
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
where
    P: harness_application::ports::conversation::ConversationProvider + 'static,
    I: ConsoleInput + ?Sized,
{
    use menu::MenuAction;
    match menu::parse_menu_choice(line) {
        MenuAction::BackToChat | MenuAction::Cancel => {
            state.menu_mode = false;
            input.update_prompt(menu::CHAT_PROMPT)
        }
        MenuAction::InvalidChoice if line.trim().parse::<usize>().is_err() => {
            state.menu_mode = false;
            input.update_prompt(menu::CHAT_PROMPT)?;
            Box::pin(process_idle_line(
                line,
                conversation,
                workspace,
                observer,
                input,
                output,
                channels,
                activity,
                policy,
                state,
            ))
            .await
        }
        MenuAction::InvalidChoice => {
            output
                .write_text("Elige una opción del 0 al 5.\n".into())
                .await
        }
        MenuAction::Exit => {
            state.exit_requested = true;
            Ok(())
        }
        MenuAction::Config => {
            output
                .write_text(config_text(
                    state,
                    *policy.lock().map_err(|_| "permission policy unavailable")?,
                ))
                .await
        }
        MenuAction::Permissions => {
            show_permissions(output, policy).await?;
            output
                .write_text("Usa /permissions auto o /permissions ask.\n".into())
                .await
        }
        MenuAction::Login if active => {
            state.menu_mode = false;
            input.update_prompt(menu::CHAT_PROMPT)?;
            state.pending_lines.push_back("/login".into());
            output
                .write_text("Inicio de sesión en cola para después del turno.\n".into())
                .await
        }
        MenuAction::NewSession if active => {
            state.menu_mode = false;
            input.update_prompt(menu::CHAT_PROMPT)?;
            state.pending_lines.push_back("/new".into());
            output
                .write_text("Nueva sesión en cola para después del turno.\n".into())
                .await
        }
        MenuAction::Login => {
            perform_login(
                conversation,
                workspace,
                observer,
                input,
                output,
                channels,
                activity,
                policy,
                state,
            )
            .await
        }
        MenuAction::NewSession => {
            state.menu_mode = false;
            state.discard_menu_choices = true;
            input.update_prompt(menu::CHAT_PROMPT)?;
            state.session_id = open_session(
                conversation,
                workspace,
                observer,
                input,
                output,
                channels,
                activity,
                policy,
                state,
            )
            .await?;
            if state.session_id.is_some() {
                output.write_text("Nueva sesión iniciada.\n".into()).await?;
            }
            state.discard_menu_choices = false;
            state
                .pending_lines
                .retain(|line| line.trim().parse::<usize>().is_err());
            Ok(())
        }
    }
}

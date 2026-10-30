use crate::terminal::{menu, session_settings};
use harness_application::conversation::Conversation;
use harness_domain::conversation::{ConversationError, PermissionPolicy};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc as tokio_mpsc;

use super::{Activity, ConsoleInput, ConsoleOutput, enqueue_image, image_path, queue_prompt};
use super::{ConsoleResult, UiChannels, UiState};
use super::{
    commands::is_setting_command,
    configuration::{config_text, set_permissions, show_permissions},
};

pub(super) async fn handle_live_line<P, I>(
    line: &str,
    conversation: &Arc<Conversation<P>>,
    input: &mut I,
    output: &Arc<dyn ConsoleOutput>,
    activity: &Arc<Activity>,
    policy: &Arc<Mutex<PermissionPolicy>>,
    state: &mut UiState,
    channels: &mut UiChannels,
    cancel_tx: &tokio_mpsc::UnboundedSender<Result<(), ConversationError>>,
    cancel_started: &mut bool,
    cancel_deadline: &mut Option<tokio::time::Instant>,
) -> ConsoleResult<()>
where
    P: harness_application::ports::conversation::ConversationProvider + 'static,
    I: ConsoleInput + ?Sized,
{
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
                    line,
                    conversation,
                    input,
                    output,
                    activity,
                    policy,
                    state,
                    channels,
                    cancel_tx,
                    cancel_started,
                    cancel_deadline,
                ))
                .await
            }
            menu::MenuAction::InvalidChoice => {
                output
                    .write_text("Elige una opción del 0 al 5.\n".into())
                    .await
            }
            menu::MenuAction::Exit => {
                state.exit_requested = true;
                super::begin_cancel(
                    conversation,
                    state,
                    cancel_tx,
                    cancel_started,
                    cancel_deadline,
                    activity,
                    input,
                )?;
                Ok(())
            }
            menu::MenuAction::Permissions => show_permissions(output, policy).await,
            menu::MenuAction::Config => {
                output
                    .write_text(config_text(
                        state,
                        *policy.lock().map_err(|_| "permission policy unavailable")?,
                    ))
                    .await
            }
            menu::MenuAction::Login | menu::MenuAction::NewSession => {
                state.menu_mode = false;
                input.update_prompt(menu::CHAT_PROMPT)?;
                state
                    .pending_lines
                    .push_back(if line.trim() == "1" { "/login" } else { "/new" }.into());
                output.write_text("Comando en cola.\n".into()).await
            }
        };
    }
    let trimmed = line.trim();
    match trimmed {
        "/usage" => output.write_text(session_settings::usage(state.usage.as_ref())).await,
        _ if is_setting_command(trimmed) => {
            state.pending_lines.push_back(trimmed.to_owned());
            output
                .write_text("Cambio de configuración en cola para después del turno.\n".into())
                .await
        }
        "/menu" => {
            state.menu_mode = true;
            input.update_prompt(menu::MENU_PROMPT)?;
            output.write_text(menu::render_menu().into()).await
        }
        "/permissions" => show_permissions(output, policy).await,
        "/permissions auto" => set_permissions(output, policy, PermissionPolicy::AutoApprove).await,
        "/permissions ask" => set_permissions(output, policy, PermissionPolicy::Ask).await,
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
        "/cancel" | "/exit" | "/quit" => {
            state.exit_requested = trimmed != "/cancel";
            super::begin_cancel(
                conversation,
                state,
                cancel_tx,
                cancel_started,
                cancel_deadline,
                activity,
                input,
            )?;
            output
                .write_text(if state.exit_requested {
                    "Cerrando después de cancelar el turno actual…\n".into()
                } else {
                    "Cancelación solicitada.\n".into()
                })
                .await
        }
        "/login" | "/new" => {
            state.pending_lines.push_back(trimmed.to_owned());
            output
                .write_text("Comando en cola para después del turno.\n".into())
                .await
        }
        _ if trimmed.starts_with("/image") => {
            if let Some(path) = image_path(line) {
                enqueue_image(path, state, &channels.image_tx, output).await
            } else {
                output.write_text("Uso: /image <ruta>.\n".into()).await
            }
        }
        _ if trimmed.starts_with('/') => output
            .write_text("Comandos: /menu, /permissions [auto|ask], /config, /image <ruta>, /cancel, /exit\n".into())
            .await,
        _ => {
            queue_prompt(line.to_owned(), state);
            output
                .write_text(format!("Mensaje en cola (#{}).\n", state.pending_prompts.len()))
                .await
        }
    }
}

use harness_application::conversation::Conversation;
use harness_domain::conversation::{AuthMethod, SessionInfo};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use super::{
    Activity, ConsoleInput, ConsoleObserver, ConsoleOutput, ConsoleResult, InputEvent, UiChannels,
    UiState, spawn_operation, wait_setup,
};
use harness_domain::conversation::PermissionPolicy;

pub(super) async fn open_session<P, I>(
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
where
    P: harness_application::ports::conversation::ConversationProvider + 'static,
    I: ConsoleInput + ?Sized,
{
    activity.start("Abriendo una sesión…")?;
    state.config_options.clear();
    state.usage = None;
    let task = spawn_operation({
        let conversation = conversation.clone();
        let workspace = workspace.to_path_buf();
        let observer = observer.clone();
        async move {
            conversation
                .start_session(&workspace, observer.as_ref())
                .await
        }
    });
    match wait_setup(task, input, output, channels, activity, policy, state).await {
        Ok(SessionInfo { id, config_options }) => {
            state.config_options = config_options;
            Ok(Some(id))
        }
        Err(message) if message == "Operación interrumpida." => Err(message),
        Err(message) if message.starts_with("Se requiere iniciar sesión:") => {
            output
                .write_text(format!("{message}. Usa /login.\n"))
                .await?;
            Ok(None)
        }
        Err(message) => {
            output
                .write_text(format!(
                    "No se pudo abrir una sesión nueva: {message}. Reintenta con /new; si hace falta, usa /login.\n"
                ))
                .await?;
            Ok(None)
        }
    }
}

pub(super) async fn perform_login<P, I>(
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
    let Some(auth_methods) = state
        .connection
        .as_ref()
        .map(|connection| connection.auth_methods.clone())
    else {
        output
            .write_text("Todavía no hay conexión ACP.\n".into())
            .await?;
        return Ok(());
    };
    let was_menu = state.menu_mode;
    state.menu_mode = false;
    if was_menu {
        input.update_prompt(super::menu::CHAT_PROMPT)?;
    }
    state.discard_menu_choices = was_menu;
    let Some(method) = choose_auth_method(&auth_methods, input, output, state).await? else {
        state.discard_menu_choices = false;
        output
            .write_text("Inicio de sesión cancelado.\n".into())
            .await?;
        return Ok(());
    };
    input.update_prompt(super::menu::CHAT_PROMPT)?;
    activity.start("Iniciando sesión…")?;
    let task = spawn_operation({
        let conversation = conversation.clone();
        let observer = observer.clone();
        let method_id = method.id;
        async move {
            conversation
                .authenticate(&method_id, observer.as_ref())
                .await
        }
    });
    match wait_setup(task, input, output, channels, activity, policy, state).await {
        Ok(connection) => {
            state.connection = Some(connection);
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
                output.write_text("Sesión iniciada.\n".into()).await?;
            }
        }
        Err(error) => {
            output
                .write_text(format!("Error de autenticación: {error}\n"))
                .await?
        }
    }
    state.discard_menu_choices = false;
    if was_menu {
        state
            .pending_lines
            .retain(|line| line.trim().parse::<usize>().is_err());
    }
    Ok(())
}

pub(super) async fn choose_auth_method<I: ConsoleInput + ?Sized>(
    methods: &[AuthMethod],
    input: &mut I,
    output: &Arc<dyn ConsoleOutput>,
    state: &mut UiState,
) -> ConsoleResult<Option<AuthMethod>> {
    if methods.is_empty() {
        output
            .write_text("El agente no anunció métodos de autenticación.\n".into())
            .await?;
        return Ok(None);
    }
    let mut text = "Métodos disponibles:\n".to_owned();
    for (index, method) in methods.iter().enumerate() {
        text.push_str(&format!("{}. {}\n", index + 1, method.name));
        if let Some(description) = &method.description {
            text.push_str(&format!("   {description}\n"));
        }
    }
    text.push_str("0. Cancelar\n");
    input.set_clipboard_available(false)?;
    output.write_text(text).await?;
    loop {
        input.update_prompt("Método de inicio de sesión > ")?;
        let line = if let Some(line) = state.pending_lines.pop_front() {
            InputEvent::Line(line)
        } else {
            input.next_event().await?
        };
        match line {
            InputEvent::Line(line) => {
                let Ok(choice) = line.trim().parse::<usize>() else {
                    state.pending_lines.push_front(line);
                    input.set_clipboard_available(true)?;
                    return Ok(None);
                };
                input.set_clipboard_available(true)?;
                return Ok(choice
                    .checked_sub(1)
                    .and_then(|index| methods.get(index))
                    .cloned());
            }
            InputEvent::PasteRequested => continue,
            InputEvent::Interrupted | InputEvent::Eof => {
                input.set_clipboard_available(true)?;
                return Ok(None);
            }
        }
    }
}

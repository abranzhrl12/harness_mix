//! Composition roots for chat and login; no command or protocol logic.
use super::terminal::{self, ConsoleIo};
use super::{ConsoleOptions, ConsoleResult};
use super::{attachments::SystemClipboardReader, context::UiRuntime};
use super::{choose_auth_method, run_loop, spawn_operation, wait_setup};
use crate::menu;
use harness_application::conversation::Conversation;
use std::{path::Path, sync::Arc};

pub async fn run_with_options<P>(
    conversation: Arc<Conversation<P>>,
    workspace: &Path,
    options: ConsoleOptions,
) -> ConsoleResult<()>
where
    P: harness_application::ports::conversation::ConversationProvider + 'static,
{
    let ConsoleIo {
        input,
        output,
        terminal,
    } = terminal::open(menu::CHAT_PROMPT)?;
    run_loop(
        conversation,
        workspace.to_path_buf(),
        input,
        output,
        options,
        terminal,
        Arc::new(SystemClipboardReader),
    )
    .await
}

pub async fn login<P: harness_application::ports::conversation::ConversationProvider + 'static>(
    conversation: &Arc<Conversation<P>>,
) -> ConsoleResult<()> {
    login_with_options(conversation, ConsoleOptions::default()).await
}

pub async fn login_with_options<
    P: harness_application::ports::conversation::ConversationProvider + 'static,
>(
    conversation: &Arc<Conversation<P>>,
    options: ConsoleOptions,
) -> ConsoleResult<()> {
    let ConsoleIo {
        mut input,
        output,
        terminal,
    } = terminal::open("Método de inicio de sesión > ")?;
    let UiRuntime {
        mut channels,
        policy,
        activity,
        observer,
        mut state,
    } = UiRuntime::new(options, terminal, Arc::new(SystemClipboardReader));
    output
        .write_text("Antigravity · iniciar sesión con ACP\n".into())
        .await?;

    activity.start("Conectando con Antigravity…")?;
    let connect = {
        let conversation = conversation.clone();
        let observer = observer.clone();
        spawn_operation(async move { conversation.connect(observer.as_ref()).await })
    };
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
    let auth_methods = state
        .connection
        .as_ref()
        .map(|info| info.auth_methods.clone())
        .unwrap_or_default();
    let Some(method) = choose_auth_method(&auth_methods, &mut *input, &output, &mut state).await?
    else {
        return Ok(());
    };
    activity.start("Iniciando sesión…")?;
    let authenticate = {
        let conversation = conversation.clone();
        let observer = observer.clone();
        let method_id = method.id;
        spawn_operation(async move {
            conversation
                .authenticate(&method_id, observer.as_ref())
                .await
        })
    };
    state.connection = Some(
        wait_setup(
            authenticate,
            &mut *input,
            &output,
            &mut channels,
            &activity,
            &policy,
            &mut state,
        )
        .await?,
    );
    output
        .write_text("Inicio de sesión completado.\n".into())
        .await
}

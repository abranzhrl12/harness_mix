//! Bounded cancellation of the current provider turn.
use super::{ConsoleInput, ConsoleResult, UiState, current_prompt};
use crate::terminal::activity::Activity;
use harness_application::conversation::Conversation;
use harness_domain::conversation::ConversationError;
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc as tokio_mpsc;

pub(super) fn start_cancel<
    P: harness_application::ports::conversation::ConversationProvider + 'static,
>(
    conversation: &Arc<Conversation<P>>,
    state: &UiState,
    tx: &tokio_mpsc::UnboundedSender<Result<(), ConversationError>>,
    started: &mut bool,
    deadline: &mut Option<tokio::time::Instant>,
) {
    if *started {
        return;
    }
    let Some(session_id) = state.session_id.clone() else {
        return;
    };
    *started = true;
    *deadline = Some(tokio::time::Instant::now() + Duration::from_secs(5));
    let conversation = conversation.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        let _ = tx.send(conversation.cancel(&session_id).await);
    });
}

pub(super) fn begin_cancel<
    P: harness_application::ports::conversation::ConversationProvider + 'static,
    I: ConsoleInput + ?Sized,
>(
    conversation: &Arc<Conversation<P>>,
    state: &UiState,
    cancel_tx: &tokio_mpsc::UnboundedSender<Result<(), ConversationError>>,
    cancel_started: &mut bool,
    cancel_deadline: &mut Option<tokio::time::Instant>,
    activity: &Arc<Activity>,
    input: &mut I,
) -> ConsoleResult<()> {
    if *cancel_started {
        return Ok(());
    }
    start_cancel(
        conversation,
        state,
        cancel_tx,
        cancel_started,
        cancel_deadline,
    );
    if !*cancel_started {
        return Ok(());
    }
    activity.stop()?;
    input.update_prompt(current_prompt(state))
}

pub(super) async fn wait_until(deadline: Option<tokio::time::Instant>) {
    if let Some(deadline) = deadline {
        tokio::time::sleep_until(deadline).await;
    } else {
        std::future::pending::<()>().await;
    }
}

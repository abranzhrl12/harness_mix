//! Prompt presentation; no protocol or terminal ownership.
use super::{ConsoleInput, ConsoleResult, UiState};
use crate::terminal::{activity::Activity, menu};
use std::sync::Arc;

pub(super) fn render_activity<I: ConsoleInput + ?Sized>(
    input: &mut I,
    activity: &Arc<Activity>,
    state: &UiState,
) -> ConsoleResult<()> {
    if let Some((frame, label)) = activity.next_frame()? {
        input.update_prompt(&format!("[{frame}] {label} | {}", current_prompt(state)))?;
    }
    Ok(())
}

pub(super) fn current_prompt(state: &UiState) -> &'static str {
    if state.menu_mode {
        menu::MENU_PROMPT
    } else {
        menu::CHAT_PROMPT
    }
}

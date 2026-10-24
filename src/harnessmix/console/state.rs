use super::UiEvent;
use super::ports::ClipboardReader;
use harness_domain::conversation::{
    ConnectionInfo, ImageAttachment, SessionConfigOption, SessionUsage,
};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::mpsc as tokio_mpsc;

#[derive(Default)]
pub(super) struct UiState {
    pub(super) connection: Option<ConnectionInfo>,
    pub(super) session_id: Option<String>,
    pub(super) config_options: Vec<SessionConfigOption>,
    pub(super) usage: Option<SessionUsage>,
    pub(super) pending_lines: VecDeque<String>,
    pub(super) pending_prompts: VecDeque<PendingPrompt>,
    pub(super) next_image_ids: Vec<u64>,
    pub(super) image_results: HashMap<u64, Result<ImageAttachment, String>>,
    pub(super) next_image_id: u64,
    pub(super) menu_mode: bool,
    pub(super) discard_menu_choices: bool,
    pub(super) exit_requested: bool,
    pub(super) cancel_requested: bool,
    pub(super) idle_interrupt_armed: bool,
}

pub(super) struct PendingPrompt {
    pub(super) text: String,
    pub(super) image_ids: Vec<u64>,
}

pub(super) type ImageCompletion = (u64, Result<ImageAttachment, String>);

pub(super) struct UiChannels {
    pub(super) clipboard: Arc<dyn ClipboardReader>,
    pub(super) events: tokio_mpsc::UnboundedReceiver<UiEvent>,
    pub(super) events_tx: tokio_mpsc::UnboundedSender<UiEvent>,
    pub(super) image_tx: tokio_mpsc::UnboundedSender<ImageCompletion>,
    pub(super) image_rx: tokio_mpsc::UnboundedReceiver<ImageCompletion>,
}

pub(super) fn interrupt_exits_when_armed(armed: &mut bool) -> bool {
    if *armed {
        true
    } else {
        *armed = true;
        false
    }
}

pub(super) fn queue_prompt(text: String, state: &mut UiState) {
    let image_ids = std::mem::take(&mut state.next_image_ids);
    state
        .pending_prompts
        .push_back(PendingPrompt { text, image_ids });
}

pub(super) fn ready_prompt(state: &mut UiState) -> Option<PendingPrompt> {
    let front = state.pending_prompts.front()?;
    if front
        .image_ids
        .iter()
        .any(|id| !state.image_results.contains_key(id))
    {
        return None;
    }
    state.pending_prompts.pop_front()
}

pub(super) fn take_images(
    state: &mut UiState,
    ids: &[u64],
) -> Result<Vec<ImageAttachment>, String> {
    let mut output = Vec::with_capacity(ids.len());
    for id in ids {
        match state.image_results.remove(id) {
            Some(Ok(image)) => output.push(image),
            Some(Err(error)) => {
                return Err(format!("Mensaje no enviado; falló el adjunto: {error}"));
            }
            None => return Err("Adjunto de imagen pendiente no disponible.".into()),
        }
    }
    Ok(output)
}

pub(super) fn remove_pending_image(id: u64, state: &mut UiState) {
    state.next_image_ids.retain(|pending| *pending != id);
    for prompt in &mut state.pending_prompts {
        prompt.image_ids.retain(|pending| *pending != id);
    }
}

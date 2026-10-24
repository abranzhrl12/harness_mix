//! Per-session composition: shares one event bus and one permission policy.
use super::ports::ClipboardReader;
use super::{ConsoleObserver, ConsoleOptions, UiChannels, UiState};
use crate::activity::Activity;
use harness_domain::conversation::PermissionPolicy;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

pub(super) struct UiRuntime {
    pub(super) channels: UiChannels,
    pub(super) policy: Arc<Mutex<PermissionPolicy>>,
    pub(super) activity: Arc<Activity>,
    pub(super) observer: Arc<ConsoleObserver>,
    pub(super) state: UiState,
}

impl UiRuntime {
    pub(super) fn new(
        options: ConsoleOptions,
        terminal: bool,
        clipboard: Arc<dyn ClipboardReader>,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (image_tx, image_rx) = mpsc::unbounded_channel();
        let channels = UiChannels {
            events: event_rx,
            events_tx: event_tx.clone(),
            image_tx,
            image_rx,
            clipboard,
        };
        let policy = Arc::new(Mutex::new(options.permissions));
        let activity = Arc::new(Activity::new(terminal && options.animate));
        let observer = Arc::new(ConsoleObserver {
            events: event_tx,
            permissions: policy.clone(),
            activity: activity.clone(),
        });
        Self {
            channels,
            policy,
            activity,
            observer,
            state: UiState::default(),
        }
    }
}

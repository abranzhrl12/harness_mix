use super::ports::ConsoleResult;
use crate::activity::Activity;
use harness_domain::conversation::{
    ConversationEvent, ConversationObserver, PermissionDecision, PermissionPolicy,
    PermissionRequest,
};
use std::sync::{Arc, Mutex, mpsc};
use tokio::sync::mpsc as tokio_mpsc;

pub(super) enum UiEvent {
    Metadata {
        event: ConversationEvent,
        ack: mpsc::SyncSender<ConsoleResult<()>>,
    },
    Output {
        text: String,
        markdown: bool,
        ack: mpsc::SyncSender<ConsoleResult<()>>,
    },
    Permission {
        request: PermissionRequest,
        reply: mpsc::SyncSender<PermissionDecision>,
    },
    Clipboard {
        id: u64,
        result: Result<Option<crate::clipboard::ClipboardContent>, String>,
    },
}

#[derive(Clone)]
pub(super) struct ConsoleObserver {
    pub(super) events: tokio_mpsc::UnboundedSender<UiEvent>,
    pub(super) permissions: Arc<Mutex<PermissionPolicy>>,
    pub(super) activity: Arc<Activity>,
}

impl ConversationObserver for ConsoleObserver {
    fn on_event(&self, event: ConversationEvent) -> Result<(), String> {
        if matches!(
            &event,
            ConversationEvent::ConfigOptions(_) | ConversationEvent::Usage(_)
        ) {
            let (ack, received) = mpsc::sync_channel(1);
            self.events
                .send(UiEvent::Metadata { event, ack })
                .map_err(|_| "console event loop closed".to_owned())?;
            return received
                .recv()
                .map_err(|_| "console metadata acknowledgement closed".to_owned())?;
        }
        let markdown = matches!(&event, ConversationEvent::Text(_));
        let Some(text) = event_text(event) else {
            return Ok(());
        };
        let resume = self.activity.pause()?;
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        if self
            .events
            .send(UiEvent::Output {
                text,
                markdown,
                ack: ack_tx,
            })
            .is_err()
        {
            self.activity.resume(resume)?;
            return Err("console event loop closed".into());
        }
        let result = ack_rx
            .recv()
            .map_err(|_| "console output acknowledgement closed".to_owned())?;
        self.activity.resume(resume)?;
        result
    }

    fn request_permission(&self, request: PermissionRequest) -> Result<PermissionDecision, String> {
        let resume = self.activity.pause()?;
        let policy = *self
            .permissions
            .lock()
            .map_err(|_| "permission policy unavailable")?;
        if let Some(decision) = policy.decision(&request) {
            let (ack_tx, ack_rx) = mpsc::sync_channel(1);
            if self
                .events
                .send(UiEvent::Output {
                    text: format!("\n{}\n", permission_action(&request, &decision)),
                    markdown: false,
                    ack: ack_tx,
                })
                .is_err()
            {
                self.activity.resume(resume)?;
                return Err("console event loop closed".into());
            }
            let result = ack_rx
                .recv()
                .map_err(|_| "console output acknowledgement closed".to_owned())?;
            self.activity.resume(resume)?;
            result?;
            return Ok(decision);
        }
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        if self
            .events
            .send(UiEvent::Permission {
                request,
                reply: reply_tx,
            })
            .is_err()
        {
            self.activity.resume(resume)?;
            return Err("console event loop closed".into());
        }
        let decision = reply_rx
            .recv()
            .map_err(|_| "permission prompt closed".to_owned())?;
        self.activity.resume(resume)?;
        Ok(decision)
    }
}

pub(super) fn event_text(event: ConversationEvent) -> Option<String> {
    match event {
        ConversationEvent::ConfigOptions(_) | ConversationEvent::Usage(_) => None,
        ConversationEvent::Text(text) => Some(text),
        ConversationEvent::Thought(_) => None,
        ConversationEvent::ToolCall { title, status } => {
            let label = match status.as_str() {
                "pending" => "Pendiente",
                "in_progress" => "En curso",
                "completed" => "Listo",
                "failed" => "Error",
                _ => "Herramienta",
            };
            let title = if title == "Agent tool" {
                "Herramienta"
            } else {
                &title
            };
            Some(format!("\n[{label}] {title}\n"))
        }
        ConversationEvent::Status(status) => Some(format!("\n{status}\n")),
        ConversationEvent::LoginUrl(url) => {
            if url
                .to_ascii_lowercase()
                .contains("continue=https%3a%2f%2fone.google.com%2fai")
            {
                None
            } else {
                Some(format!(
                    "\nAbre esta dirección para iniciar sesión con Google:\n{url}\n"
                ))
            }
        }
    }
}

pub(super) fn permission_action(
    request: &PermissionRequest,
    decision: &PermissionDecision,
) -> String {
    match decision {
        PermissionDecision::Selected(id) => {
            let action = request
                .options
                .iter()
                .find(|item| &item.id == id)
                .map(|item| item.name.as_str())
                .unwrap_or("acción aprobada");
            format!("Permiso aprobado automáticamente: {action}.")
        }
        PermissionDecision::Cancelled => {
            "Permiso denegado automáticamente; no hay opción de aprobación disponible.".into()
        }
    }
}

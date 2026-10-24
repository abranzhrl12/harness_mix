use harness_domain::conversation::{
    ConnectionInfo, ConversationError, ConversationObserver, ImageAttachment, SessionConfigOption,
    SessionInfo,
};
use std::{future::Future, path::Path, pin::Pin};

pub type ConversationFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ConversationError>> + Send + 'a>>;

/// ACP or another agent transport implements this application boundary.
pub trait ConversationProvider: Send + Sync {
    fn initialize<'a>(
        &'a self,
        observer: &'a dyn ConversationObserver,
    ) -> ConversationFuture<'a, ConnectionInfo>;

    fn authenticate<'a>(
        &'a self,
        method_id: &'a str,
        observer: &'a dyn ConversationObserver,
    ) -> ConversationFuture<'a, ConnectionInfo>;

    fn new_session<'a>(
        &'a self,
        workspace: &'a Path,
        observer: &'a dyn ConversationObserver,
    ) -> ConversationFuture<'a, SessionInfo>;

    fn set_config_option<'a>(
        &'a self,
        session_id: &'a str,
        config_id: &'a str,
        value: &'a str,
        observer: &'a dyn ConversationObserver,
    ) -> ConversationFuture<'a, Vec<SessionConfigOption>> {
        let _ = (session_id, config_id, value, observer);
        Box::pin(async {
            Err(ConversationError::Other(
                "El proveedor no admite configuración de sesión".into(),
            ))
        })
    }

    fn prompt<'a>(
        &'a self,
        session_id: &'a str,
        text: &'a str,
        observer: &'a dyn ConversationObserver,
    ) -> ConversationFuture<'a, String>;

    fn prompt_with_images<'a>(
        &'a self,
        session_id: &'a str,
        text: &'a str,
        images: &'a [ImageAttachment],
        observer: &'a dyn ConversationObserver,
    ) -> ConversationFuture<'a, String> {
        if images.is_empty() {
            self.prompt(session_id, text, observer)
        } else {
            Box::pin(async {
                Err(ConversationError::Other(
                    "El proveedor de conversación no admite imágenes adjuntas".into(),
                ))
            })
        }
    }

    fn cancel<'a>(&'a self, session_id: &'a str) -> ConversationFuture<'a, ()>;
    fn close<'a>(&'a self) -> ConversationFuture<'a, ()>;
}

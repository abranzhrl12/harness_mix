use crate::ports::conversation::ConversationProvider;
use harness_domain::conversation::{
    ConnectionInfo, ConversationError, ConversationObserver, ImageAttachment, SessionConfigOption,
    SessionInfo,
};
use std::path::Path;

/// Orchestrates the interactive conversation while leaving ACP details in its adapter.
pub struct Conversation<P> {
    provider: P,
}

impl<P: ConversationProvider> Conversation<P> {
    pub fn new(provider: P) -> Self {
        Self { provider }
    }

    pub async fn connect(
        &self,
        observer: &dyn ConversationObserver,
    ) -> Result<ConnectionInfo, ConversationError> {
        self.provider.initialize(observer).await
    }

    pub async fn authenticate(
        &self,
        method_id: &str,
        observer: &dyn ConversationObserver,
    ) -> Result<ConnectionInfo, ConversationError> {
        self.provider.authenticate(method_id, observer).await
    }

    pub async fn start_session(
        &self,
        workspace: &Path,
        observer: &dyn ConversationObserver,
    ) -> Result<SessionInfo, ConversationError> {
        self.provider.new_session(workspace, observer).await
    }

    pub async fn set_config_option(
        &self,
        session_id: &str,
        config_id: &str,
        value: &str,
        observer: &dyn ConversationObserver,
    ) -> Result<Vec<SessionConfigOption>, ConversationError> {
        self.provider
            .set_config_option(session_id, config_id, value, observer)
            .await
    }

    /// Sends one prompt and returns the protocol's stop reason after streamed events.
    pub async fn send(
        &self,
        session_id: &str,
        text: &str,
        observer: &dyn ConversationObserver,
    ) -> Result<String, ConversationError> {
        self.provider.prompt(session_id, text, observer).await
    }

    /// Sends text with image attachments through providers that support them.
    pub async fn send_with_images(
        &self,
        session_id: &str,
        text: &str,
        images: &[ImageAttachment],
        observer: &dyn ConversationObserver,
    ) -> Result<String, ConversationError> {
        self.provider
            .prompt_with_images(session_id, text, images, observer)
            .await
    }

    pub async fn cancel(&self, session_id: &str) -> Result<(), ConversationError> {
        self.provider.cancel(session_id).await
    }

    pub async fn close(&self) -> Result<(), ConversationError> {
        self.provider.close().await
    }
}

#[cfg(test)]
mod tests {
    use super::Conversation;
    use crate::ports::conversation::{ConversationFuture, ConversationProvider};
    use harness_domain::conversation::{
        AuthMethod, ConnectionInfo, ConversationError, ConversationEvent, ConversationObserver,
        ImageAttachment, PermissionDecision, PermissionOption, PermissionRequest, SessionInfo,
    };
    use std::{
        future::Future,
        path::Path,
        sync::{Arc, Mutex},
        task::{Context, Poll, Wake, Waker},
    };

    struct FakeProvider(Arc<Mutex<Vec<String>>>);

    impl ConversationProvider for FakeProvider {
        fn initialize<'a>(
            &'a self,
            observer: &'a dyn ConversationObserver,
        ) -> ConversationFuture<'a, ConnectionInfo> {
            Box::pin(async move {
                observer
                    .on_event(ConversationEvent::Status("connected".into()))
                    .map_err(ConversationError::Other)?;
                Ok(ConnectionInfo {
                    agent_name: "fake-agent".into(),
                    auth_methods: vec![AuthMethod {
                        id: "google".into(),
                        name: "Google".into(),
                        description: None,
                    }],
                })
            })
        }

        fn authenticate<'a>(
            &'a self,
            method_id: &'a str,
            _observer: &'a dyn ConversationObserver,
        ) -> ConversationFuture<'a, ConnectionInfo> {
            Box::pin(async move {
                self.0.lock().unwrap().push(format!("auth:{method_id}"));
                Ok(ConnectionInfo {
                    agent_name: "fake-agent".into(),
                    auth_methods: Vec::new(),
                })
            })
        }

        fn new_session<'a>(
            &'a self,
            workspace: &'a Path,
            _observer: &'a dyn ConversationObserver,
        ) -> ConversationFuture<'a, SessionInfo> {
            Box::pin(async move {
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("session:{}", workspace.display()));
                Ok(SessionInfo {
                    id: "s-1".into(),
                    config_options: Vec::new(),
                })
            })
        }

        fn prompt<'a>(
            &'a self,
            session_id: &'a str,
            text: &'a str,
            observer: &'a dyn ConversationObserver,
        ) -> ConversationFuture<'a, String> {
            Box::pin(async move {
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("prompt:{session_id}:{text}"));
                observer
                    .on_event(ConversationEvent::Text("answer".into()))
                    .map_err(ConversationError::Other)?;
                let decision = observer
                    .request_permission(PermissionRequest {
                        title: "Run command?".into(),
                        options: vec![PermissionOption {
                            id: "allow".into(),
                            name: "Allow".into(),
                            kind: "allow_once".into(),
                        }],
                    })
                    .map_err(ConversationError::Other)?;
                if decision != PermissionDecision::Selected("allow".into()) {
                    return Err(ConversationError::Other(
                        "permission was not granted".into(),
                    ));
                }
                Ok("end_turn".into())
            })
        }

        fn cancel<'a>(&'a self, session_id: &'a str) -> ConversationFuture<'a, ()> {
            Box::pin(async move {
                self.0.lock().unwrap().push(format!("cancel:{session_id}"));
                Ok(())
            })
        }

        fn close<'a>(&'a self) -> ConversationFuture<'a, ()> {
            Box::pin(async move {
                self.0.lock().unwrap().push("close".into());
                Ok(())
            })
        }
    }

    #[derive(Default)]
    struct RecordingObserver {
        events: Mutex<Vec<ConversationEvent>>,
    }

    impl ConversationObserver for RecordingObserver {
        fn on_event(&self, event: ConversationEvent) -> Result<(), String> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }

        fn request_permission(
            &self,
            request: PermissionRequest,
        ) -> Result<PermissionDecision, String> {
            Ok(PermissionDecision::Selected(request.options[0].id.clone()))
        }
    }

    struct ReadyWake;
    impl Wake for ReadyWake {
        fn wake(self: Arc<Self>) {}
    }

    fn run_ready<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(ReadyWake));
        let mut context = Context::from_waker(&waker);
        let mut future = Box::pin(future);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("fake provider should only return immediately ready futures"),
        }
    }

    #[test]
    fn use_case_forwards_auth_streamed_events_permissions_and_session_lifecycle() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let conversation = Conversation::new(FakeProvider(calls.clone()));
        let observer = RecordingObserver::default();

        let connection = run_ready(conversation.connect(&observer)).unwrap();
        assert_eq!(connection.agent_name, "fake-agent");
        assert_eq!(connection.auth_methods[0].id, "google");
        run_ready(conversation.authenticate("google", &observer)).unwrap();
        let session = run_ready(conversation.start_session(Path::new("/repo"), &observer)).unwrap();
        assert_eq!(session.id, "s-1");
        assert_eq!(
            run_ready(conversation.send(&session.id, "hello", &observer)).unwrap(),
            "end_turn"
        );
        run_ready(conversation.cancel(&session.id)).unwrap();
        run_ready(conversation.close()).unwrap();

        assert_eq!(
            *calls.lock().unwrap(),
            [
                "auth:google",
                "session:/repo",
                "prompt:s-1:hello",
                "cancel:s-1",
                "close"
            ]
        );
        assert_eq!(
            *observer.events.lock().unwrap(),
            [
                ConversationEvent::Status("connected".into()),
                ConversationEvent::Text("answer".into())
            ]
        );
    }

    #[test]
    fn image_send_delegates_empty_attachments_and_keeps_unsupported_providers_compatible() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let conversation = Conversation::new(FakeProvider(calls.clone()));
        let observer = RecordingObserver::default();
        let image = ImageAttachment {
            mime_type: "image/png".into(),
            data: b"png".to_vec(),
        };

        assert_eq!(
            run_ready(conversation.send_with_images("s-1", "hello", &[], &observer)).unwrap(),
            "end_turn"
        );
        let error = run_ready(conversation.send_with_images(
            "s-1",
            "describe this",
            &[image],
            &observer,
        ))
        .unwrap_err();
        assert_eq!(
            error,
            ConversationError::Other(
                "El proveedor de conversación no admite imágenes adjuntas".into()
            )
        );
        assert_eq!(*calls.lock().unwrap(), ["prompt:s-1:hello"]);
    }

    #[test]
    fn providers_without_session_configuration_keep_the_default_unsupported_result() {
        let conversation = Conversation::new(FakeProvider(Arc::new(Mutex::new(Vec::new()))));
        let observer = RecordingObserver::default();

        assert_eq!(
            run_ready(conversation.set_config_option("s-1", "model", "unknown", &observer))
                .unwrap_err(),
            ConversationError::Other(
                "El proveedor no admite configuración de sesión".into()
            )
        );
    }
}

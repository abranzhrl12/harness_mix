use std::{future::Future, pin::Pin};

pub type ConsoleResult<T> = Result<T, String>;

pub(super) type LocalFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

pub(super) type ClipboardFuture<'a> =
    Pin<Box<dyn Future<Output = ConsoleResult<Option<ClipboardContent>>> + Send + 'a>>;

/// The port owns its payload; native adapters depend on this contract.
#[derive(Debug)]
pub(crate) enum ClipboardContent {
    Image(harness_domain::conversation::ImageAttachment),
    Text(String),
}

/// Attachment acquisition is injected; orchestration never reads the OS clipboard.
pub(super) trait ClipboardReader: Send + Sync {
    fn read(&self) -> ClipboardFuture<'_>;
}

#[derive(Debug)]
pub(super) enum InputEvent {
    Line(String),
    PasteRequested,
    Eof,
    Interrupted,
}

pub(super) trait ConsoleInput {
    fn next_event(&mut self) -> LocalFuture<'_, ConsoleResult<InputEvent>>;
    fn update_prompt(&mut self, prompt: &str) -> ConsoleResult<()>;

    fn set_clipboard_available(&mut self, _available: bool) -> ConsoleResult<()> {
        Ok(())
    }

    fn insert_text(&mut self, _text: &str) -> ConsoleResult<()> {
        Err("Este adaptador de entrada no admite pegar texto.".into())
    }

    fn set_selector_mode(&mut self, _active: bool, _prompt: &str) -> ConsoleResult<bool> {
        Ok(false)
    }
}

pub(super) trait ConsoleOutput {
    fn write_text(&self, text: String) -> LocalFuture<'_, ConsoleResult<()>>;

    fn write_markdown(&self, text: String) -> LocalFuture<'_, ConsoleResult<()>> {
        self.write_text(text)
    }

    fn flush_response(&self) -> LocalFuture<'_, ConsoleResult<()>> {
        Box::pin(async { Ok(()) })
    }
}

use super::ports::{ConsoleOutput, ConsoleResult, LocalFuture};
use crate::rendering::{self, MessageRenderer};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

/// Presentation decorator that frames assistant chunks before rendering Markdown.
pub(super) struct RenderingOutput {
    inner: Arc<dyn ConsoleOutput>,
    renderer: Arc<dyn MessageRenderer>,
    pending: Mutex<ResponseStream>,
    terminal: bool,
}

#[derive(Default)]
pub(super) struct ResponseStream {
    framing: crate::streaming::StreamBuffer,
    ready: VecDeque<String>,
    render_state: rendering::RenderState,
}

impl RenderingOutput {
    #[cfg(test)]
    pub(super) fn new(inner: Arc<dyn ConsoleOutput>, renderer: Arc<dyn MessageRenderer>) -> Self {
        Self::for_terminal(inner, renderer, true)
    }

    pub(super) fn for_terminal(
        inner: Arc<dyn ConsoleOutput>,
        renderer: Arc<dyn MessageRenderer>,
        terminal: bool,
    ) -> Self {
        let pending = ResponseStream {
            framing: if terminal {
                crate::streaming::StreamBuffer::complete_lines()
            } else {
                crate::streaming::StreamBuffer::default()
            },
            ..ResponseStream::default()
        };
        Self {
            inner,
            renderer,
            pending: Mutex::new(pending),
            terminal,
        }
    }

    async fn emit_ready(&self) -> ConsoleResult<()> {
        loop {
            let markdown = self
                .pending
                .lock()
                .map_err(|_| "response buffer unavailable")?
                .ready
                .pop_front();
            let Some(markdown) = markdown else {
                return Ok(());
            };
            let mut next_state = self
                .pending
                .lock()
                .map_err(|_| "response buffer unavailable")?
                .render_state
                .clone();
            let mut rendered = self.renderer.render_progressive(
                &markdown,
                rendering::terminal_width(),
                &mut next_state,
            );
            // SharedWriter needs complete lines to preserve the editable prompt.
            if self.terminal && !rendered.is_empty() && !rendered.ends_with('\n') {
                rendered.push('\n');
            }
            if let Err(error) = self.inner.write_text(rendered).await {
                self.pending
                    .lock()
                    .map_err(|_| "response buffer unavailable")?
                    .ready
                    .push_front(markdown);
                return Err(error);
            }
            self.pending
                .lock()
                .map_err(|_| "response buffer unavailable")?
                .render_state = next_state;
        }
    }
}

impl ConsoleOutput for RenderingOutput {
    fn write_markdown(&self, text: String) -> LocalFuture<'_, ConsoleResult<()>> {
        Box::pin(async move {
            {
                let mut pending = self
                    .pending
                    .lock()
                    .map_err(|_| "response buffer unavailable")?;
                let segments = pending.framing.push(&text);
                pending.ready.extend(segments);
            }
            self.emit_ready().await
        })
    }

    fn write_text(&self, text: String) -> LocalFuture<'_, ConsoleResult<()>> {
        self.inner.write_text(rendering::sanitize(&text))
    }

    fn flush_response(&self) -> LocalFuture<'_, ConsoleResult<()>> {
        Box::pin(async move {
            {
                let mut pending = self
                    .pending
                    .lock()
                    .map_err(|_| "response buffer unavailable")?;
                if let Some(tail) = pending.framing.finish() {
                    pending.ready.push_back(tail);
                }
            }
            self.emit_ready().await?;
            self.pending
                .lock()
                .map_err(|_| "response buffer unavailable")?
                .render_state = rendering::RenderState::default();
            Ok(())
        })
    }
}

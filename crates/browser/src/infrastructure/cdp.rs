//! Synchronous BrowserBackend bridge to the dedicated Tokio actor.

use std::{
    sync::{Mutex, mpsc},
    time::Duration,
};

use harness_domain::{
    Failure,
    browser::{BrowserArgs, BrowserResult},
    execution::OperationContext,
};
use tokio::sync::mpsc as tokio_mpsc;

use crate::{
    application::BrowserBackend,
    infrastructure::actor::{ActorHandle, Request, spawn_actor},
};

pub(super) const CHANNEL_CAPACITY: usize = 32;
pub(super) const MAX_CONNECTIONS: usize = 8;
pub(super) const MAX_TARGETS: usize = 128;
pub(super) const MAX_ATTACHED_PAGES: usize = 32;
pub(super) const MAX_EVENT_STACK: usize = 4_096;
pub(super) const DEFAULT_HTML_CHARS: usize = 24_000;
pub(super) const MAX_SCREENSHOT_BYTES: usize = 4 * 1024 * 1024;
pub(super) const MAX_SCREENSHOT_DIMENSION: u32 = 4_096;
pub(super) const OPERATION_TIMEOUT: Duration = Duration::from_secs(20);

/// Lazy actor-backed implementation that owns no browser handles itself.
pub(crate) struct CdpBrowserBackend {
    actor: Mutex<Option<ActorHandle>>,
}

impl CdpBrowserBackend {
    pub(crate) fn new() -> Self {
        Self {
            actor: Mutex::new(None),
        }
    }

    fn actor(&self) -> Result<ActorHandle, Failure> {
        let mut slot = self.actor.lock().map_err(|_| {
            (
                "BROWSER_RUNTIME",
                "No se pudo inicializar el actor del navegador",
            )
        })?;
        if slot.is_none() {
            *slot = Some(spawn_actor()?);
        }
        slot.as_ref().cloned().ok_or((
            "BROWSER_RUNTIME",
            "No se pudo iniciar el actor del navegador",
        ))
    }
}

impl BrowserBackend for CdpBrowserBackend {
    fn execute(
        &self,
        args: &BrowserArgs,
        context: &OperationContext,
    ) -> Result<BrowserResult, Failure> {
        context.check()?;
        let (reply, result) = mpsc::sync_channel(1);
        self.actor()?
            .sender
            .try_send(Request {
                args: args.clone(),
                context: context.clone(),
                reply,
            })
            .map_err(|error| match error {
                tokio_mpsc::error::TrySendError::Full(_) => (
                    "BROWSER_BUSY",
                    "El navegador tiene demasiadas solicitudes pendientes",
                ),
                tokio_mpsc::error::TrySendError::Closed(_) => (
                    "BROWSER_RUNTIME",
                    "El actor del navegador ya no está disponible",
                ),
            })?;
        loop {
            context.check()?;
            match result.recv_timeout(Duration::from_millis(20)) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err((
                        "BROWSER_RUNTIME",
                        "El actor del navegador terminó inesperadamente",
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "cdp_tests.rs"]
mod tests;

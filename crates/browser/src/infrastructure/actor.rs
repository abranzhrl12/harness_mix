use std::{
    collections::HashMap,
    future::Future,
    sync::mpsc::{self, SyncSender},
    thread,
    time::Duration,
};

use harness_domain::{
    Failure,
    browser::{BrowserArgs, BrowserResult},
    execution::OperationContext,
};
use tokio::{runtime::Builder, sync::mpsc as tokio_mpsc, time};

use crate::infrastructure::{
    cdp::{CHANNEL_CAPACITY, OPERATION_TIMEOUT},
    connection::BrowserConnection,
    dispatch,
};

#[derive(Clone)]
pub(super) struct ActorHandle {
    pub(super) sender: tokio_mpsc::Sender<Request>,
}

pub(super) struct Request {
    pub(super) args: BrowserArgs,
    pub(super) context: OperationContext,
    pub(super) reply: SyncSender<Result<BrowserResult, Failure>>,
}

pub(super) fn spawn_actor() -> Result<ActorHandle, Failure> {
    let (sender, receiver) = tokio_mpsc::channel(CHANNEL_CAPACITY);
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("harness-browser-cdp".into())
        .spawn(move || {
            match Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(runtime) => {
                    let _ = ready_tx.send(Ok(()));
                    runtime.block_on(run_actor(receiver));
                }
                Err(_) => {
                    let _ = ready_tx.send(Err(()));
                }
            }
        })
        .map_err(|_| ("BROWSER_RUNTIME", "No se pudo crear el hilo del navegador"))?;
    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| {
            (
                "BROWSER_RUNTIME",
                "El runtime del navegador no inició a tiempo",
            )
        })?
        .map_err(|_| {
            (
                "BROWSER_RUNTIME",
                "No se pudo iniciar el runtime Tokio del navegador",
            )
        })?;
    Ok(ActorHandle { sender })
}

async fn run_actor(mut receiver: tokio_mpsc::Receiver<Request>) {
    let mut connections = HashMap::<String, BrowserConnection>::new();
    let mut sessions = HashMap::<String, String>::new();
    while let Some(request) = receiver.recv().await {
        let context = request.context.clone();
        let result = bounded(
            context,
            dispatch::dispatch(
                request.args,
                request.context,
                &mut connections,
                &mut sessions,
            ),
        )
        .await;
        let _ = request.reply.send(result);
    }
}

async fn bounded<F, T>(context: OperationContext, future: F) -> Result<T, Failure>
where
    F: Future<Output = Result<T, Failure>>,
{
    tokio::pin!(future);
    let timeout = time::sleep(OPERATION_TIMEOUT);
    tokio::pin!(timeout);
    let mut poll_cancel = time::interval(Duration::from_millis(20));
    loop {
        tokio::select! {
            biased;
            result = &mut future => return result,
            _ = &mut timeout => return Err(("TIMEOUT", "La operación del navegador superó el plazo")),
            _ = poll_cancel.tick() => context.check()?,
        }
    }
}

use crate::adapters::tasks::TaskManager;
use std::{error::Error, future::Future, io, time::Duration};

const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(10);
const SHUTDOWN_ERROR: &str =
    "Background job workers did not drain within the 10-second shutdown deadline";

pub async fn drain_task_manager(tasks: Option<&TaskManager>) -> Result<(), String> {
    if let Some(tasks) = tasks {
        if !tasks.shutdown_and_wait(SHUTDOWN_DEADLINE).await {
            return Err(SHUTDOWN_ERROR.into());
        }
    }
    Ok(())
}

pub async fn drain_managers(managers: &[TaskManager]) -> Result<(), String> {
    // Signal every manager before waiting: one slow project must not leave
    // other projects admitting work or running uncancelled jobs.
    for manager in managers {
        manager.shutdown();
    }
    let mut failed = false;
    for manager in managers {
        if !manager.shutdown_and_wait(SHUTDOWN_DEADLINE).await {
            failed = true;
        }
    }
    if failed {
        Err(SHUTDOWN_ERROR.into())
    } else {
        Ok(())
    }
}

pub async fn startup_error<E>(
    error: E,
    drain: impl Future<Output = Result<(), String>>,
) -> Box<dyn Error>
where
    E: Error + Send + Sync + 'static,
{
    match drain.await {
        Ok(()) => Box::new(error),
        Err(drain_error) => Box::new(io::Error::other(format!("{error}; {drain_error}"))),
    }
}

pub async fn run_server<S, StartError, WaitFuture, WaitError, Drain>(
    started: Result<S, StartError>,
    waiting: impl FnOnce(S) -> WaitFuture,
    drain: Drain,
) -> Result<(), Box<dyn Error>>
where
    StartError: Error + Send + Sync + 'static,
    WaitFuture: Future<Output = Result<(), WaitError>>,
    WaitError: Error + Send + Sync + 'static,
    Drain: Future<Output = Result<(), String>>,
{
    let service = match started {
        Ok(service) => service,
        Err(error) => return Err(startup_error(error, drain).await),
    };

    let result = waiting(service).await;
    match (result, drain.await) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(Box::new(error)),
        (Ok(()), Err(error)) => Err(Box::new(io::Error::other(error))),
        (Err(error), Err(drain_error)) => Err(Box::new(io::Error::other(format!(
            "{error}; {drain_error}"
        )))),
    }
}

#[cfg(test)]
#[path = "coordinator_tests.rs"]
mod tests;

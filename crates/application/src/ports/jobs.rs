use harness_domain::{Failure, tasks::TaskArgs};
use serde_json::Value;
use std::{future::Future, pin::Pin};

pub type JobFuture<'a> = Pin<Box<dyn Future<Output = Result<Value, Failure>> + Send + 'a>>;
pub type JobEventFuture<'a> = Pin<Box<dyn Future<Output = JobEvent> + Send + 'a>>;

/// Application boundary for background jobs. Transport and runtime adapters can
/// dispatch jobs and forward progress without depending on the task manager.
pub trait JobService: Send + Sync {
    fn handle(&self, args: TaskArgs) -> JobFuture<'_>;
    fn subscribe(&self) -> Box<dyn JobEventStream>;
}

pub trait JobEventStream: Send {
    fn recv(&mut self) -> JobEventFuture<'_>;
}

pub enum JobEvent {
    Event(Value),
    Lagged(u64),
    Closed,
}

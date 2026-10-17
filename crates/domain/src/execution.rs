//! Cooperative cancellation and deadlines, independent of any async runtime.
use crate::Failure;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct OperationContext {
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
}
impl Default for OperationContext {
    fn default() -> Self {
        Self::with_timeout(DEFAULT_TIMEOUT)
    }
}
impl OperationContext {
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            deadline: Instant::now() + timeout,
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn check(&self) -> Result<(), Failure> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(("CANCELLED", "operación cancelada"));
        }
        if Instant::now() >= self.deadline {
            return Err(("TIMEOUT", "operación superó el plazo"));
        }
        Ok(())
    }
}

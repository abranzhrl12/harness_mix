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

pub fn new_request_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_deadlines_keep_owner_cancellation_and_never_extend_parent() {
        let owner = OperationContext::without_deadline();
        let timed = owner.with_execution_timeout(Some(Duration::ZERO));
        assert_eq!(timed.check().unwrap_err().0, "TIMEOUT");
        assert!(owner.check().is_ok());
        owner.cancel();
        assert_eq!(timed.check().unwrap_err().0, "CANCELLED");

        let expired = OperationContext::with_timeout(Duration::ZERO);
        let child = expired.with_execution_timeout(Some(Duration::from_secs(30)));
        assert_eq!(child.check().unwrap_err().0, "TIMEOUT");
        assert_eq!(
            expired.with_execution_timeout(None).check().unwrap_err().0,
            "TIMEOUT"
        );
    }
}
pub fn valid_request_id(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.hyphenated().to_string() == value)
}

#[derive(Clone)]
pub struct OperationContext {
    cancelled: Arc<AtomicBool>,
    deadline: Option<Instant>,
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
            deadline: Some(Instant::now() + timeout),
        }
    }
    /// Background work lives independently of an individual MCP request.
    /// Cancellation remains available even when no execution deadline is set.
    pub fn without_deadline() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            deadline: None,
        }
    }
    /// Apply an execution deadline while keeping cancellation shared with the owner.
    pub fn with_execution_timeout(&self, timeout: Option<Duration>) -> Self {
        let deadline = timeout.map(|value| Instant::now() + value);
        Self {
            cancelled: self.cancelled.clone(),
            deadline: match (self.deadline, deadline) {
                (Some(existing), Some(new)) => Some(existing.min(new)),
                (existing, None) => existing,
                (None, new) => new,
            },
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn check(&self) -> Result<(), Failure> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(("CANCELLED", "operación cancelada"));
        }
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(("TIMEOUT", "operación superó el plazo"));
        }
        Ok(())
    }
}

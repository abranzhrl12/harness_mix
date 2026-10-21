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
    progress: Option<Arc<dyn Fn(ProgressUpdate<'_>) + Send + Sync>>,
}

/// Operation-local telemetry. Counts describe this phase, not global index coverage.
/// Borrowed paths avoid allocating when reporting each file; observers must not block.
pub struct ProgressUpdate<'a> {
    pub phase: &'static str,
    pub completed: usize,
    pub total: Option<usize>,
    pub current: Option<&'a str>,
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
            progress: None,
        }
    }
    /// Background work lives independently of an individual MCP request.
    /// Cancellation remains available even when no execution deadline is set.
    pub fn without_deadline() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            deadline: None,
            progress: None,
        }
    }
    /// Apply an execution deadline while keeping cancellation shared with the owner.
    pub fn with_execution_timeout(&self, timeout: Option<Duration>) -> Self {
        let deadline = timeout.map(|value| Instant::now() + value);
        Self {
            cancelled: self.cancelled.clone(),
            progress: self.progress.clone(),
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
    pub fn with_progress(
        mut self,
        observer: impl Fn(ProgressUpdate<'_>) + Send + Sync + 'static,
    ) -> Self {
        self.progress = Some(Arc::new(observer));
        self
    }
    pub fn report_progress(
        &self,
        phase: &'static str,
        completed: usize,
        total: Option<usize>,
        current: Option<&str>,
    ) {
        if let Some(observer) = &self.progress {
            observer(ProgressUpdate {
                phase,
                completed,
                total,
                current,
            });
        }
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

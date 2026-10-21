//! Lifecycle rules for durable background jobs.
//!
//! This module contains only state and transition policy. Persistence, clocks,
//! scheduling, and execution remain in the adapters.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    RetryWait,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    Interrupted,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::RetryWait => "retry_wait",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::TimedOut | Self::Interrupted
        )
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "retry_wait" => Some(Self::RetryWait),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "timed_out" => Some(Self::TimedOut),
            "interrupted" => Some(Self::Interrupted),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobLifecycle {
    status: JobStatus,
    attempt: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionError {
    InvalidTransition,
    InvalidAttempt,
}

impl JobLifecycle {
    pub const fn queued() -> Self {
        Self {
            status: JobStatus::Queued,
            attempt: 0,
        }
    }

    pub const fn status(&self) -> JobStatus {
        self.status
    }

    pub const fn attempt(&self) -> u8 {
        self.attempt
    }

    /// Reconstructs any state accepted by the existing durable snapshot format.
    /// The adapter supplies the configured maximum because it is stored beside
    /// the lifecycle in the persisted job configuration.
    pub fn restore(status: &str, attempt: u8, maximum: u8) -> Option<Self> {
        let status = JobStatus::parse(status)?;
        if attempt > 3 || !(1..=3).contains(&maximum) || attempt > maximum {
            return None;
        }
        Some(Self { status, attempt })
    }

    /// Marks an accepted persisted active state interrupted without changing
    /// its attempt count. Snapshot compatibility intentionally permits some
    /// state/attempt pairs that a newly executed job would never produce.
    pub fn interrupt_for_recovery(&mut self) -> bool {
        if self.status.is_terminal() {
            return false;
        }
        self.status = JobStatus::Interrupted;
        true
    }

    /// Applies a lifecycle change made by the scheduler or executor.
    /// Restart recovery uses `interrupt_for_recovery` to preserve legacy states.
    pub fn transition(&mut self, next: JobStatus, attempt: u8) -> Result<(), TransitionError> {
        if attempt > 3 {
            return Err(TransitionError::InvalidAttempt);
        }
        let allowed = match (self.status, next) {
            (JobStatus::Queued, JobStatus::Running | JobStatus::Failed | JobStatus::Cancelled) => {
                true
            }
            (
                JobStatus::Running,
                JobStatus::RetryWait
                | JobStatus::Succeeded
                | JobStatus::Failed
                | JobStatus::Cancelled
                | JobStatus::TimedOut,
            ) => true,
            (
                JobStatus::RetryWait,
                JobStatus::Running | JobStatus::Failed | JobStatus::Cancelled,
            ) => true,
            _ => false,
        };
        if !allowed {
            return Err(TransitionError::InvalidTransition);
        }
        let attempt_valid = match (self.status, next) {
            (JobStatus::Queued, JobStatus::Running) => attempt == 1,
            (JobStatus::Queued, JobStatus::Failed) => attempt == 1,
            (JobStatus::RetryWait, JobStatus::Running | JobStatus::Failed) => {
                attempt == self.attempt.saturating_add(1)
            }
            (JobStatus::Queued, JobStatus::Cancelled) => attempt == 0,
            (JobStatus::RetryWait, JobStatus::Cancelled) => attempt == self.attempt,
            (
                JobStatus::Running,
                JobStatus::RetryWait
                | JobStatus::Succeeded
                | JobStatus::Failed
                | JobStatus::Cancelled
                | JobStatus::TimedOut,
            ) => attempt == self.attempt,
            _ => false,
        };
        if !attempt_valid {
            return Err(TransitionError::InvalidAttempt);
        }
        self.status = next;
        self.attempt = attempt;
        Ok(())
    }
}

//! Durable scheduling and lifecycle management for background verification jobs.
mod engine;
mod store;

pub use engine::{PreparedVerification, TaskManager, VerificationExecutor};
pub use store::DirectoryPinner;

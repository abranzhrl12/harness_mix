//! Native session state, persistence, and in-process lifecycle.
mod manager;
mod model;
mod persistence;
mod validation;

pub use manager::Sessions;
pub(super) use manager::now;
// Preserve the original session model paths for internal consumers.
#[allow(unused_imports)]
pub use model::{Declared, Session, State};
pub(super) use persistence::persist_snapshot;
pub(super) use validation::validate;

#[cfg(test)]
mod tests;

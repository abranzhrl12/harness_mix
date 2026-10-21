//! PostgreSQL catalog access. Only fixed, parameterized metadata queries are exposed.
mod catalog;
mod config;
mod connection;
mod operations;
#[path = "postgresql_pool.rs"]
mod pool;

pub(in crate::adapters) use config::enabled;
pub(in crate::adapters) use operations::{describe, search, status};

//! PostgreSQL catalog access. Only fixed, parameterized metadata queries are exposed.
#[cfg(feature = "sql-tools")]
mod catalog;
mod config;
#[cfg(feature = "sql-tools")]
mod connection;
#[cfg(feature = "sql-tools")]
mod operations;
#[cfg(feature = "sql-tools")]
#[path = "postgresql_pool.rs"]
mod pool;

#[cfg(feature = "sql-tools")]
pub(in crate::adapters) use config::enabled;
#[cfg(feature = "sql-tools")]
pub(in crate::adapters) use operations::{describe, map, search, status};

#[cfg(not(feature = "sql-tools"))]
pub(in crate::adapters) fn status(
    _project: &crate::adapters::project::Project,
    _args: &harness_domain::sql::SqlStatusArgs,
    _context: &harness_domain::execution::OperationContext,
) -> Result<serde_json::Value, harness_domain::Failure> {
    Err((
        "SQL_DISABLED",
        "SQL tools are disabled (sql-tools feature not enabled)",
    ))
}

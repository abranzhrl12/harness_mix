//! SQL Server native adapter module powered by Tiberius.
//! Reads configuration from .agents/harness.config.json or Web.config,
//! and handles sql_status, sql_search, sql_describe, and sql_query.

#[path = "sql/config.rs"]
mod config;
#[cfg(feature = "sql-tools")]
#[path = "sql/connection.rs"]
mod connection;
#[cfg(feature = "sql-tools")]
#[path = "sql/diagnostics.rs"]
mod diagnostics;
#[cfg(feature = "sql-tools")]
#[path = "sql/engine.rs"]
mod engine;
#[cfg(feature = "sql-tools")]
#[path = "sql/operations.rs"]
mod operations;
#[cfg(feature = "sql-tools")]
#[path = "sql_parameters.rs"]
mod parameters;
#[cfg(feature = "sql-tools")]
#[path = "sql/presentation.rs"]
mod presentation;

pub use config::{SqlConfig, load_sql_config};
#[cfg(feature = "sql-tools")]
pub(super) use connection::block_on_safe;
#[cfg(feature = "sql-tools")]
pub use connection::resolve_connection_string;

use crate::adapters::project::Project;
use harness_domain::{
    Failure,
    execution::OperationContext,
    sql::{SqlDescribeArgs, SqlMapArgs, SqlQueryArgs, SqlSearchArgs, SqlStatusArgs},
};
use serde_json::Value;

#[cfg(not(feature = "sql-tools"))]
const SQL_DISABLED_ERROR: Failure = (
    "SQL_DISABLED",
    "SQL tools are disabled (sql-tools feature not enabled)",
);

#[cfg(feature = "sql-tools")]
pub fn status(
    project: &Project,
    args: &SqlStatusArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    match engine::select_engine(project, args.engine)? {
        harness_domain::sql::SqlEngine::SqlServer => operations::status(project, args, context),
        harness_domain::sql::SqlEngine::Postgresql => {
            crate::adapters::postgresql::status(project, args, context)
        }
    }
}

#[cfg(not(feature = "sql-tools"))]
pub fn status(
    _project: &Project,
    _args: &SqlStatusArgs,
    _context: &OperationContext,
) -> Result<Value, Failure> {
    Err(SQL_DISABLED_ERROR)
}

#[cfg(feature = "sql-tools")]
pub fn search(
    project: &Project,
    args: &SqlSearchArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    match engine::select_engine(project, args.engine)? {
        harness_domain::sql::SqlEngine::SqlServer => operations::search(project, args, context),
        harness_domain::sql::SqlEngine::Postgresql => {
            crate::adapters::postgresql::search(project, args, context)
        }
    }
}

#[cfg(not(feature = "sql-tools"))]
pub fn search(
    _project: &Project,
    _args: &SqlSearchArgs,
    _context: &OperationContext,
) -> Result<Value, Failure> {
    Err(SQL_DISABLED_ERROR)
}

#[cfg(feature = "sql-tools")]
pub fn map(
    project: &Project,
    args: &SqlMapArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    match engine::select_engine(project, args.engine)? {
        harness_domain::sql::SqlEngine::SqlServer => operations::map(project, args, context),
        harness_domain::sql::SqlEngine::Postgresql => {
            crate::adapters::postgresql::map(project, args, context)
        }
    }
}

#[cfg(not(feature = "sql-tools"))]
pub fn map(
    _project: &Project,
    _args: &SqlMapArgs,
    _context: &OperationContext,
) -> Result<Value, Failure> {
    Err(SQL_DISABLED_ERROR)
}

#[cfg(feature = "sql-tools")]
pub fn describe(
    project: &Project,
    args: &SqlDescribeArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    match engine::select_engine(project, args.engine)? {
        harness_domain::sql::SqlEngine::SqlServer => operations::describe(project, args, context),
        harness_domain::sql::SqlEngine::Postgresql => {
            crate::adapters::postgresql::describe(project, args, context)
        }
    }
}

#[cfg(not(feature = "sql-tools"))]
pub fn describe(
    _project: &Project,
    _args: &SqlDescribeArgs,
    _context: &OperationContext,
) -> Result<Value, Failure> {
    Err(SQL_DISABLED_ERROR)
}

#[cfg(feature = "sql-tools")]
pub fn query(
    project: &Project,
    args: &SqlQueryArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    match engine::select_engine(project, args.engine)? {
        harness_domain::sql::SqlEngine::SqlServer => operations::query(project, args, context),
        harness_domain::sql::SqlEngine::Postgresql => Err((
            "PG_QUERY_UNAVAILABLE",
            "PostgreSQL currently supports status/search/describe only; arbitrary queries are not enabled",
        )),
    }
}

#[cfg(not(feature = "sql-tools"))]
pub fn query(
    _project: &Project,
    _args: &SqlQueryArgs,
    _context: &OperationContext,
) -> Result<Value, Failure> {
    Err(SQL_DISABLED_ERROR)
}

#[cfg(all(test, feature = "sql-tools"))]
#[path = "sql/tests.rs"]
mod safety_tests;

//! SQL Server native adapter module powered by Tiberius.
//! Reads configuration from .agents/harness.config.json or Web.config,
//! and handles sql_status, sql_search, sql_describe, and sql_query.

#[path = "sql/config.rs"]
mod config;
#[path = "sql/connection.rs"]
mod connection;
#[path = "sql/diagnostics.rs"]
mod diagnostics;
#[path = "sql/engine.rs"]
mod engine;
#[path = "sql/operations.rs"]
mod operations;
#[path = "sql_parameters.rs"]
mod parameters;

pub use config::{SqlConfig, load_sql_config};
pub(super) use connection::block_on_safe;
pub use connection::resolve_connection_string;

use crate::adapters::project::Project;
use harness_domain::{
    Failure,
    execution::OperationContext,
    sql::{SqlDescribeArgs, SqlMapArgs, SqlQueryArgs, SqlSearchArgs, SqlStatusArgs},
};
use serde_json::Value;

pub fn status(
    project: &Project,
    args: &SqlStatusArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    operations::status(project, args, context)
}

pub fn search(
    project: &Project,
    args: &SqlSearchArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    operations::search(project, args, context)
}

pub fn map(
    project: &Project,
    args: &SqlMapArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    operations::map(project, args, context)
}

pub fn describe(
    project: &Project,
    args: &SqlDescribeArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    operations::describe(project, args, context)
}

pub fn query(
    project: &Project,
    args: &SqlQueryArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    operations::query(project, args, context)
}

#[cfg(test)]
#[path = "sql/tests.rs"]
mod safety_tests;

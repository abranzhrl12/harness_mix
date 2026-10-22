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
#[path = "sql/presentation.rs"]
mod presentation;

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
    match engine::select_engine(project, args.engine)? {
        harness_domain::sql::SqlEngine::SqlServer => operations::status(project, args, context),
        harness_domain::sql::SqlEngine::Postgresql => {
            crate::adapters::postgresql::status(project, args, context)
        }
    }
}

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

#[cfg(test)]
#[path = "sql/tests.rs"]
mod safety_tests;

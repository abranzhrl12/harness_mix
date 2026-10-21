use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

/// Structured database access through engine adapters.
pub trait SqlAccess: Send + Sync {
    fn sql_status(
        &self,
        _args: &harness_domain::sql::SqlStatusArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "SQL status adapter unavailable"))
    }
    fn sql_search(
        &self,
        _args: &harness_domain::sql::SqlSearchArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "SQL search adapter unavailable"))
    }
    fn sql_describe(
        &self,
        _args: &harness_domain::sql::SqlDescribeArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "SQL describe adapter unavailable"))
    }
    fn sql_query(
        &self,
        _args: &harness_domain::sql::SqlQueryArgs,
        _context: &OperationContext,
    ) -> Result<Value, Failure> {
        Err(("UNSUPPORTED", "SQL query adapter unavailable"))
    }
}

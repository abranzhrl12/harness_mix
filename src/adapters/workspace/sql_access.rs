use crate::adapters::{NativeWorkspace, sql};
use harness_application::ports::workspace::SqlAccess;
use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

impl SqlAccess for NativeWorkspace {
    fn sql_status(
        &self,
        args: &harness_domain::sql::SqlStatusArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        sql::status(&self.project, args, context)
    }
    fn sql_search(
        &self,
        args: &harness_domain::sql::SqlSearchArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        sql::search(&self.project, args, context)
    }
    fn sql_map(
        &self,
        args: &harness_domain::sql::SqlMapArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        sql::map(&self.project, args, context)
    }
    fn sql_describe(
        &self,
        args: &harness_domain::sql::SqlDescribeArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        sql::describe(&self.project, args, context)
    }
    fn sql_query(
        &self,
        args: &harness_domain::sql::SqlQueryArgs,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        sql::query(&self.project, args, context)
    }
}

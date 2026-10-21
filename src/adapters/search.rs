use super::project::{Failure, Project};
mod graph;
mod plan;
mod presentation;
mod scan;
#[cfg(test)]
mod tests;
use harness_domain::{contracts::SearchArgs, execution::OperationContext};
use serde_json::Value;

#[cfg(test)]
pub fn search(project: &Project, args: &SearchArgs) -> Result<Value, Failure> {
    search_with_context(project, args, &OperationContext::default())
}
pub fn search_with_context(
    project: &Project,
    args: &SearchArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    let plan = plan::build(project, args)?;
    let outcome = scan::execute(project, args, context, plan)?;
    let outcome = scan::finalize(outcome, args, project, context);
    presentation::present(args, outcome)
}

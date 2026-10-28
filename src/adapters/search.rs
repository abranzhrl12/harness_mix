use super::project::{Failure, Project};
#[cfg(test)]
mod budget_tests;
#[cfg(test)]
mod cache_integration_tests;
mod graph;
mod matcher_cache;
mod plan;
mod presentation;
mod ranking;
mod scan;
mod structural;
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
    let context = context.with_execution_timeout(Some(scan::SCAN_TIMEOUT));
    let mut outcome = scan::execute(project, args, &context, plan)?;
    let mut ranked = false;
    if outcome.all.is_empty()
        && !outcome.bounded
        && !outcome.timed_out
        && !outcome.cancelled
        && outcome.skipped == 0
    {
        if let Some(matcher) = plan::diacritic_matcher(args)? {
            let fallback = plan::build_diacritic_fallback(project, args, matcher)?;
            let first_started = outcome.started;
            let first_bytes = outcome.bytes;
            let first_visited = outcome.visited;
            outcome = scan::execute(project, args, &context, fallback)?;
            outcome.started = first_started;
            outcome.bytes += first_bytes;
            outcome.visited += first_visited;
        }
    }
    if outcome.all.is_empty()
        && !outcome.bounded
        && !outcome.timed_out
        && !outcome.cancelled
        && outcome.skipped == 0
    {
        if let Some(matcher) = ranking::fallback_matcher(args)? {
            let fallback = plan::build_fallback(project, args, matcher)?;
            let first_started = outcome.started;
            let first_bytes = outcome.bytes;
            let first_visited = outcome.visited;
            outcome = scan::execute(project, args, &context, fallback)?;
            outcome.started = first_started;
            outcome.bytes += first_bytes;
            outcome.visited += first_visited;
            ranking::rank(&mut outcome.all, &args.query, &context)?;
            ranked = true;
        }
    }
    let outcome = scan::finalize(outcome, args, project, &context);
    let mut result = presentation::present(args, outcome)?;
    if ranked {
        result["retrieval"] = serde_json::json!({"mode":"terms","ranking":"bm25","exactMatches":0});
        result["contextLines"] = serde_json::json!(args.context_lines.unwrap_or(2));
        result["_detail"]["retrieval"] = result["retrieval"].clone();
    }
    Ok(result)
}

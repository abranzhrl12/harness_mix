use super::config::load_sql_config;
use crate::adapters::project::Project;
use harness_domain::Failure;

pub(super) fn select_engine(
    project: &Project,
    requested: Option<harness_domain::sql::SqlEngine>,
) -> Result<harness_domain::sql::SqlEngine, Failure> {
    if let Some(engine) = requested {
        return Ok(engine);
    }
    let pg = crate::adapters::postgresql::enabled(project)?;
    let ms = load_sql_config(project).enabled;
    select_from_availability(pg, ms)
}

fn select_from_availability(pg: bool, ms: bool) -> Result<harness_domain::sql::SqlEngine, Failure> {
    use harness_domain::sql::SqlEngine;
    match (pg, ms) {
        (true, true) => Err((
            "SQL_ENGINE_REQUIRED",
            "Both databases are enabled; specify engine=sqlserver or postgresql",
        )),
        (true, false) => Ok(SqlEngine::Postgresql),
        _ => Ok(SqlEngine::SqlServer),
    }
}

#[cfg(test)]
#[path = "engine/tests.rs"]
mod tests;

//! Optional configuration: detect supported engines from named connection variables, never print values.
use super::{project::Project, sql_credentials};
use harness_domain::{Failure, execution::OperationContext, sql::SqlEngine};
use serde_json::Value;

pub(super) fn discover(
    project: &Project,
    wanted: Option<SqlEngine>,
) -> Result<Option<(SqlEngine, String)>, Failure> {
    let context = OperationContext::default();
    let config = project.target_optional(".agents/harness.config.json")?;
    if config.exists() {
        let bytes =
            project.bytes_with_limit(".agents/harness.config.json", &context, 1024 * 1024)?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| ("SQL_CONFIG", "Invalid project configuration JSON"))?;
        // Explicit engine sections, including enabled:false, always override automatic discovery.
        if value.get("postgresql").is_some()
            || value.get("sqlServer").is_some()
            || value.pointer("/database/autoDetect") == Some(&Value::Bool(false))
        {
            return Ok(None);
        }
    }
    let names = [
        "HARNESS_PG_CONNECTION_STRING",
        "HARNESS_SQL_CONNECTION_STRING",
        "DATABASE_URL",
    ];
    for name in names {
        match std::env::var(name) {
            Ok(value) => {
                let engine = classify(&value).ok_or((
                    "SQL_ENGINE_UNSUPPORTED",
                    "Use a PostgreSQL URI or SQL Server ADO connection string",
                ))?;
                if wanted.is_none_or(|wanted| wanted == engine) {
                    return Ok(Some((engine, name.into())));
                }
            }
            Err(std::env::VarError::NotPresent) => {}
            _ => {
                return Err((
                    "SQL_CREDENTIALS",
                    "Connection environment variable is invalid",
                ));
            }
        }
    }
    let text = if let Some(bytes) = project.credential_bytes(".env", &context)? {
        Some(
            String::from_utf8(bytes)
                .map_err(|_| ("SQL_ENV_FILE", "Project dotenv must be UTF-8"))?,
        )
    } else {
        None
    };
    for name in names {
        let value = match std::env::var(name) {
            Ok(_) => None,
            Err(std::env::VarError::NotPresent) => text
                .as_deref()
                .map(|text| sql_credentials::parse(text, name))
                .transpose()?
                .flatten(),
            _ => {
                return Err((
                    "SQL_CREDENTIALS",
                    "Connection environment variable is invalid",
                ));
            }
        };
        if let Some(value) = value {
            let engine = classify(&value).ok_or(("SQL_ENGINE_UNSUPPORTED", "Connection must be a PostgreSQL URI (postgresql:// or postgres://) or SQL Server ADO string (Server=...;Database=...); use project JSON for explicit configuration"))?;
            if wanted.is_none_or(|wanted| wanted == engine) {
                return Ok(Some((engine, name.into())));
            }
        }
    }
    Ok(None)
}
fn classify(value: &str) -> Option<SqlEngine> {
    let lower = value.trim().to_ascii_lowercase();
    if lower.starts_with("postgresql://") || lower.starts_with("postgres://") {
        return Some(SqlEngine::Postgresql);
    }
    if lower.contains("://") {
        return None;
    }
    if lower
        .split(';')
        .filter_map(|part| part.split_once('='))
        .any(|(key, _)| matches!(key.trim(), "server" | "data source" | "address"))
    {
        return Some(SqlEngine::SqlServer);
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_supported_formats_without_guessing_other_engines() {
        assert_eq!(
            classify("postgresql://user:private@localhost/db"),
            Some(SqlEngine::Postgresql)
        );
        assert_eq!(
            classify("Server=localhost;Database=test;Password=private"),
            Some(SqlEngine::SqlServer)
        );
        assert_eq!(classify("mysql://localhost/db"), None);
    }
}

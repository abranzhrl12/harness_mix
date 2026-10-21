use super::super::project::Project;
use harness_domain::Failure;
use serde::Deserialize;
use serde_json::{Value, json};

pub(super) const CONFIG: Failure = (
    "PG_CONFIG",
    "Invalid postgresql configuration; check limits, schemas, TLS and environment connection source",
);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Config {
    #[serde(skip)]
    pub(super) fingerprint: String,
    #[serde(default)]
    pub(super) enabled: bool,
    #[serde(default = "default_schema")]
    pub(super) default_schema: String,
    #[serde(default = "default_schemas")]
    pub(super) allowed_schemas: Vec<String>,
    #[serde(default = "default_rows")]
    pub(super) max_rows: usize,
    #[serde(default = "default_chars")]
    pub(super) max_chars: usize,
    #[serde(default = "default_timeout")]
    pub(super) timeout_seconds: u64,
    #[serde(default)]
    pub(super) schema_cache_seconds: Option<u64>,
    pub(super) connection: Option<Connection>,
    #[serde(default)]
    pub(super) tls: Tls,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Connection {
    pub(super) source: String,
    pub(super) name: String,
    #[serde(default = "default_env_file")]
    pub(super) env_file: Option<String>,
}
fn default_env_file() -> Option<String> {
    Some(".env".into())
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Tls {
    pub(super) mode: String,
    pub(super) ca_file: Option<String>,
}
impl Default for Tls {
    fn default() -> Self {
        Self {
            mode: "disable".into(),
            ca_file: None,
        }
    }
}
fn default_schema() -> String {
    "public".into()
}
fn default_schemas() -> Vec<String> {
    vec![default_schema()]
}
fn default_rows() -> usize {
    50
}
fn default_chars() -> usize {
    8000
}
fn default_timeout() -> u64 {
    15
}

pub(super) fn load(project: &Project) -> Result<Option<Config>, Failure> {
    let path = project.target_optional(".agents/harness.config.json")?;
    let bytes = match std::fs::read(&path) {
        Ok(bytes) if bytes.len() <= 1024 * 1024 => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => b"{}".to_vec(),
        _ => return Err(CONFIG),
    };
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| CONFIG)?;
    let pg = match value.get("postgresql") {
        Some(pg) => pg.clone(),
        None => match super::super::sql_discovery::discover(
            project,
            Some(harness_domain::sql::SqlEngine::Postgresql),
        )? {
            Some((_, name)) => {
                json!({"enabled":true,"connection":{"source":"environment","name":name}})
            }
            None => return Ok(None),
        },
    };
    let mut config: Config = serde_json::from_value(pg).map_err(|_| CONFIG)?;
    config.fingerprint = harness_domain::hash(&bytes);
    if !(1..=200).contains(&config.max_rows)
        || !(1000..=32000).contains(&config.max_chars)
        || !(1..=120).contains(&config.timeout_seconds)
        || config.schema_cache_seconds.is_some_and(|ttl| ttl > 300)
        || config.allowed_schemas.is_empty()
        || config.allowed_schemas.len() > 16
        || !config.allowed_schemas.contains(&config.default_schema)
        || config
            .allowed_schemas
            .iter()
            .any(|s| s.is_empty() || s.len() > 63 || s.contains('\0'))
        || !["verify-full", "disable", "disable-loopback"].contains(&config.tls.mode.as_str())
        || (config.tls.mode != "verify-full" && config.tls.ca_file.is_some())
    {
        return Err(CONFIG);
    }
    Ok(Some(config))
}
pub(in crate::adapters) fn enabled(project: &Project) -> Result<bool, Failure> {
    Ok(load(project)?.is_some_and(|c| c.enabled))
}
pub(super) fn required(project: &Project) -> Result<Config, Failure> {
    load(project)?
        .filter(|c| c.enabled)
        .ok_or(("PG_CONFIG", "PostgreSQL is not enabled"))
}

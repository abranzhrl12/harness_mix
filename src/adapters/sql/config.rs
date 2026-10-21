use crate::adapters::project::Project;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct SqlConfig {
    pub enabled: bool,
    pub backend: String,
    pub default_schema: String,
    pub connection_source: String,
    pub connection_path: Option<String>,
    pub connection_name: Option<String>,
    pub env_file: Option<String>,
    pub max_rows: usize,
    pub timeout_seconds: u64,
    pub max_chars: usize,
    pub schema_cache_seconds: u64,
}

impl Default for SqlConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: "tiberius".into(),
            default_schema: "dbo".into(),
            connection_source: "webConfig".into(),
            connection_path: Some("Web.config".into()),
            connection_name: Some("EFAST".into()),
            env_file: Some(".env".into()),
            max_rows: 50,
            timeout_seconds: 15,
            max_chars: 8000,
            schema_cache_seconds: 300,
        }
    }
}

pub fn load_sql_config(project: &Project) -> SqlConfig {
    let config_path = project.root.join(".agents/harness.config.json");
    let Ok(bytes) = std::fs::read(config_path) else {
        return auto_config(project);
    };
    let Ok(val) = serde_json::from_slice::<Value>(&bytes) else {
        return SqlConfig::default();
    };

    let Some(sql_val) = val.get("sqlServer") else {
        return auto_config(project);
    };

    let enabled = sql_val
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let backend = sql_val
        .get("backend")
        .and_then(|v| v.as_str())
        .unwrap_or("tiberius")
        .to_string();
    let default_schema = sql_val
        .get("defaultSchema")
        .and_then(|v| v.as_str())
        .unwrap_or("dbo")
        .to_string();
    let max_rows = sql_val
        .get("maxRows")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .clamp(1, 200) as usize;
    let timeout_seconds = sql_val
        .get("timeoutSeconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(15)
        .clamp(1, 120);
    let max_chars = sql_val
        .get("maxChars")
        .and_then(|v| v.as_u64())
        .unwrap_or(8000)
        .clamp(1000, 32000) as usize;
    let schema_cache_seconds = sql_val
        .get("schemaCacheSeconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(300);

    let conn = sql_val.get("connection");
    let connection_source = conn
        .and_then(|c| c.get("source"))
        .and_then(|v| v.as_str())
        .unwrap_or("webConfig")
        .to_string();
    let connection_path = conn
        .and_then(|c| c.get("path"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let connection_name = conn
        .and_then(|c| c.get("name"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    SqlConfig {
        enabled,
        backend,
        default_schema,
        connection_source,
        connection_path,
        connection_name,
        env_file: match conn.and_then(|c| c.get("envFile")) {
            Some(Value::Null) => None,
            Some(Value::String(file)) => Some(file.clone()),
            _ => Some(".env".into()),
        },
        max_rows,
        timeout_seconds,
        max_chars,
        schema_cache_seconds,
    }
}

fn auto_config(project: &Project) -> SqlConfig {
    let mut config = SqlConfig::default();
    if let Ok(Some((_, name))) = crate::adapters::sql_discovery::discover(
        project,
        Some(harness_domain::sql::SqlEngine::SqlServer),
    ) {
        config.enabled = true;
        config.connection_source = "environment".into();
        config.connection_name = Some(name);
    }
    config
}

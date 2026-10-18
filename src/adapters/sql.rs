//! SQL Server native adapter module powered by Tiberius.
//! Reads configuration from .agents/harness.config.json or Web.config,
//! and handles sql_status, sql_search, sql_describe, and sql_query.

use crate::adapters::project::Project;
use harness_domain::{
    Failure,
    execution::OperationContext,
    sql::{
        SqlDescribeArgs, SqlDescribeKind, SqlObjectType, SqlQueryArgs, SqlSearchArgs,
        SqlStatusArgs, assert_sql_read_only,
    },
};
use serde_json::{Value, json};
use std::collections::HashMap;
use tiberius::{AuthMethod, Client, Config};
use tokio::net::TcpStream;
use tokio_util::compat::TokioAsyncWriteCompatExt;

pub(super) fn block_on_safe<F, T>(future: F) -> T
where
    F: std::future::Future<Output = T>,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(future)),
        Err(_) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build standalone runtime for batch thread");
            rt.block_on(future)
        }
    }
}

fn select_engine(
    project: &Project,
    requested: Option<harness_domain::sql::SqlEngine>,
) -> Result<harness_domain::sql::SqlEngine, Failure> {
    use harness_domain::sql::SqlEngine;
    if let Some(engine) = requested {
        return Ok(engine);
    }
    let pg = super::postgresql::enabled(project)?;
    let ms = load_sql_config(project).enabled;
    match (pg, ms) {
        (true, true) => Err((
            "SQL_ENGINE_REQUIRED",
            "Both databases are enabled; specify engine=sqlserver or postgresql",
        )),
        (true, false) => Ok(SqlEngine::Postgresql),
        _ => Ok(SqlEngine::SqlServer),
    }
}

#[derive(Debug, Clone)]
pub struct SqlConfig {
    pub enabled: bool,
    pub backend: String,
    pub default_schema: String,
    pub connection_source: String,
    pub connection_path: Option<String>,
    pub connection_name: Option<String>,
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
        return SqlConfig::default();
    };
    let Ok(val) = serde_json::from_slice::<Value>(&bytes) else {
        return SqlConfig::default();
    };

    let Some(sql_val) = val.get("sqlServer") else {
        return SqlConfig::default();
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
        .unwrap_or(50) as usize;
    let timeout_seconds = sql_val
        .get("timeoutSeconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(15);
    let max_chars = sql_val
        .get("maxChars")
        .and_then(|v| v.as_u64())
        .unwrap_or(8000) as usize;
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
        max_rows,
        timeout_seconds,
        max_chars,
        schema_cache_seconds,
    }
}

/// Extract connection string from Web.config or environment
pub fn resolve_connection_string(project: &Project, config: &SqlConfig) -> Result<String, Failure> {
    if config.connection_source == "environment" {
        if let Ok(val) = std::env::var("HARNESS_SQL_CONNECTION_STRING") {
            return Ok(val);
        }
        // Fallback: check project root .env if not injected into process env
        let env_path = project.root.join(".env");
        if let Ok(content) = std::fs::read_to_string(env_path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("HARNESS_SQL_CONNECTION_STRING=") {
                    let cleaned = rest.trim().trim_matches('"').trim_matches('\'');
                    return Ok(cleaned.to_string());
                }
            }
        }
        return Err((
            "SQL_CONFIG",
            "Variable de entorno HARNESS_SQL_CONNECTION_STRING no definida ni encontrada en .env",
        ));
    }

    let rel_path = config.connection_path.as_deref().unwrap_or("Web.config");
    let target_name = config.connection_name.as_deref().unwrap_or("EFAST");
    let path = project.root.join(rel_path);

    let content =
        std::fs::read_to_string(&path).map_err(|_| ("SQL_CONFIG", "No se pudo leer Web.config"))?;

    let doc = roxmltree::Document::parse(&content)
        .map_err(|_| ("SQL_CONFIG", "Web.config tiene formato XML inválido"))?;

    for node in doc.descendants() {
        if node.has_tag_name("add")
            && let (Some(name), Some(conn_str)) =
                (node.attribute("name"), node.attribute("connectionString"))
            && name.eq_ignore_ascii_case(target_name)
        {
            return Ok(conn_str.to_string());
        }
    }

    Err((
        "SQL_CONFIG",
        "Connection string no encontrada en Web.config",
    ))
}

/// Parse standard ADO.NET connection string into Tiberius Config
fn parse_connection_string(conn_str: &str) -> Result<Config, Failure> {
    let mut parts = HashMap::new();
    for part in conn_str.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((k, v)) = part.split_once('=') {
            parts.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }

    let server_raw = parts
        .get("data source")
        .or_else(|| parts.get("server"))
        .or_else(|| parts.get("address"))
        .ok_or((
            "SQL_CONFIG",
            "Falta Data Source o Server en la cadena de conexión",
        ))?;

    let mut config = Config::new();

    let (host, port) = if let Some((h, p)) = server_raw.split_once(',') {
        (h.trim(), p.trim().parse::<u16>().unwrap_or(1433))
    } else if let Some((h, p)) = server_raw.split_once(':') {
        (h.trim(), p.trim().parse::<u16>().unwrap_or(1433))
    } else {
        (server_raw.as_str(), 1433)
    };

    config.host(host);
    config.port(port);

    if let Some(db) = parts
        .get("initial catalog")
        .or_else(|| parts.get("database"))
    {
        config.database(db);
    }

    let integrated = parts
        .get("integrated security")
        .map(|s| s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("sspi"))
        .unwrap_or(false);

    if integrated {
        #[cfg(windows)]
        {
            config.authentication(AuthMethod::Integrated);
        }
        #[cfg(not(windows))]
        {
            return Err((
                "SQL_CONFIG",
                "Integrated Security solo está soportada en Windows",
            ));
        }
    } else {
        let user = parts
            .get("user id")
            .or_else(|| parts.get("uid"))
            .or_else(|| parts.get("user"));
        let password = parts.get("password").or_else(|| parts.get("pwd"));
        if let (Some(u), Some(p)) = (user, password) {
            config.authentication(AuthMethod::sql_server(u, p));
        }
    }

    config.trust_cert();
    Ok(config)
}

/// Establish an active connection to SQL Server via Tiberius
async fn connect(
    project: &Project,
    config: &SqlConfig,
) -> Result<Client<tokio_util::compat::Compat<TcpStream>>, Failure> {
    let conn_str = resolve_connection_string(project, config)?;
    let t_config = parse_connection_string(&conn_str)?;

    let tcp = TcpStream::connect(t_config.get_addr()).await.map_err(|e| {
        eprintln!("SQL connection failed: {e}");
        (
            "SQL_CONNECTION",
            "No se pudo conectar al servidor SQL; verifique la IP y puerto",
        )
    })?;

    let client = Client::connect(t_config, tcp.compat_write())
        .await
        .map_err(|e| {
            eprintln!("SQL handshake/login failed: {e}");
            (
                "SQL_AUTH",
                "Fallo de autenticación o protocolo al conectar a SQL Server",
            )
        })?;

    Ok(client)
}

pub fn status(
    project: &Project,
    args: &SqlStatusArgs,
    _context: &OperationContext,
) -> Result<Value, Failure> {
    if select_engine(project, args.engine)? == harness_domain::sql::SqlEngine::Postgresql {
        return super::postgresql::status(project, args, _context);
    }
    let config = load_sql_config(project);
    if !config.enabled {
        return Ok(json!({
            "configured": false,
            "status": "not_configured"
        }));
    }

    let should_connect = args.connect.unwrap_or(true);
    let mut connection_verified = false;

    if should_connect {
        let project_clone = project.clone();
        let config_clone = config.clone();
        let res = block_on_safe(async move {
            let mut client = connect(&project_clone, &config_clone).await?;
            let row = client
                .query("SELECT 1 AS connection_ok", &[])
                .await
                .map_err(|_| ("SQL_QUERY", "SELECT 1 ping failed"))?
                .into_row()
                .await
                .map_err(|_| ("SQL_QUERY", "Fallo al leer fila de prueba"))?;
            if row.is_some() {
                Ok::<(), Failure>(())
            } else {
                Err(("SQL_QUERY", "SELECT 1 no devolvió filas"))
            }
        });
        connection_verified = res.is_ok();
    }

    Ok(json!({
        "configured": true,
        "status": if !should_connect || connection_verified { "ready" } else { "verification_failed" },
        "connected": connection_verified,
        "backend": "tiberius (native Rust)",
        "defaultSchema": config.default_schema,
        "connectionSource": config.connection_source,
        "connectionName": config.connection_name,
        "maxRows": config.max_rows,
        "timeoutSeconds": config.timeout_seconds,
        "readOnly": true,
        "permissionBoundary": "SQL login permissions"
    }))
}

pub fn search(
    project: &Project,
    args: &SqlSearchArgs,
    _context: &OperationContext,
) -> Result<Value, Failure> {
    if select_engine(project, args.engine)? == harness_domain::sql::SqlEngine::Postgresql {
        return super::postgresql::search(project, args, _context);
    }
    let config = load_sql_config(project);
    if !config.enabled {
        return Err((
            "SQL_CONFIG",
            "SQL Server no está habilitado en harness.config.json",
        ));
    }

    let query = args.query.trim();
    if query.is_empty() || query.len() > 128 {
        return Err(("ARGUMENT", "Buscar esquema con 1–128 caracteres"));
    }

    let limit = args.limit.unwrap_or(20).clamp(1, 50);
    let schema = args.schema.as_deref().unwrap_or(&config.default_schema);

    let types_clause = match args.object_type {
        Some(SqlObjectType::Table) => "'U'",
        Some(SqlObjectType::View) => "'V'",
        Some(SqlObjectType::Procedure) => "'P'",
        Some(SqlObjectType::Function) => "'FN','IF','TF'",
        Some(SqlObjectType::Column) => "'U','V'",
        None => "'U','V','P','FN','IF','TF'",
    };

    let sql = format!(
        "SELECT TOP ({}) s.name AS [schema], o.name, o.type_desc AS kind \
         FROM sys.objects AS o \
         JOIN sys.schemas AS s ON s.schema_id = o.schema_id \
         WHERE o.is_ms_shipped = 0 \
           AND o.type IN ({}) \
           AND (s.name = @P1 OR @P1 = '') \
           AND (o.name LIKE @P2 ESCAPE '~' OR EXISTS (SELECT 1 FROM sys.columns AS c WHERE c.object_id = o.object_id AND c.name LIKE @P2 ESCAPE '~')) \
         ORDER BY s.name, o.name",
        limit, types_clause
    );

    let escaped_query = format!("%{}%", query.replace(['~', '%', '_', '['], "~$0"));
    let schema_param = schema.to_string();

    let project_clone = project.clone();
    let config_clone = config.clone();

    let rows_data = block_on_safe(async move {
        let mut client = connect(&project_clone, &config_clone).await?;
        let stream = client
            .query(sql.as_str(), &[&schema_param, &escaped_query])
            .await
            .map_err(|e| {
                eprintln!("Search query error: {e}");
                ("SQL_QUERY", "Error al ejecutar búsqueda en catálogo SQL")
            })?;

        let rows = stream
            .into_first_result()
            .await
            .map_err(|_| ("SQL_QUERY", "Error al procesar filas de búsqueda"))?;

        let mut list = Vec::new();
        for r in rows {
            let schema: Option<&str> = r.get(0);
            let name: Option<&str> = r.get(1);
            let kind: Option<&str> = r.get(2);
            list.push(json!({
                "schema": schema.unwrap_or(""),
                "name": name.unwrap_or(""),
                "kind": kind.unwrap_or("")
            }));
        }
        Ok::<Vec<Value>, Failure>(list)
    })?;

    Ok(json!({
        "results": rows_data,
        "count": rows_data.len(),
        "query": query,
        "schema": schema
    }))
}

pub fn describe(
    project: &Project,
    args: &SqlDescribeArgs,
    _context: &OperationContext,
) -> Result<Value, Failure> {
    if select_engine(project, args.engine)? == harness_domain::sql::SqlEngine::Postgresql {
        return super::postgresql::describe(project, args, _context);
    }
    let config = load_sql_config(project);
    if !config.enabled {
        return Err((
            "SQL_CONFIG",
            "SQL Server no está habilitado en harness.config.json",
        ));
    }

    if args.name.trim().is_empty() || args.name.len() > 256 {
        return Err(("ARGUMENT", "Indicar nombre del objeto SQL"));
    }

    let kind = args.kind.as_ref().unwrap_or(&SqlDescribeKind::Relation);
    let name_str = args.name.clone();
    let project_clone = project.clone();
    let config_clone = config.clone();

    let result_data = block_on_safe(async move {
        let mut client = connect(&project_clone, &config_clone).await?;
        match kind {
            SqlDescribeKind::Routine => {
                let sql = "SELECT OBJECT_DEFINITION(OBJECT_ID(@P1)) AS def";
                let stream = client
                    .query(sql, &[&name_str])
                    .await
                    .map_err(|_| ("SQL_QUERY", "Fallo al consultar definición de rutina"))?;
                let rows = stream
                    .into_first_result()
                    .await
                    .map_err(|_| ("SQL_QUERY", "Fallo al leer resultado"))?;
                let def: Option<&str> = rows.first().and_then(|r| r.get(0));
                Ok::<Value, Failure>(json!({
                    "name": name_str,
                    "kind": "routine",
                    "definition": def.unwrap_or("No definition available")
                }))
            }
            SqlDescribeKind::Relation => {
                let sql = "SELECT COLUMN_NAME, DATA_TYPE, CHARACTER_MAXIMUM_LENGTH, IS_NULLABLE \
                               FROM INFORMATION_SCHEMA.COLUMNS \
                               WHERE TABLE_NAME = @P1 \
                               ORDER BY ORDINAL_POSITION";
                let stream = client
                    .query(sql, &[&name_str])
                    .await
                    .map_err(|_| ("SQL_QUERY", "Fallo al consultar columnas de tabla"))?;
                let rows = stream
                    .into_first_result()
                    .await
                    .map_err(|_| ("SQL_QUERY", "Fallo al leer resultado"))?;
                let mut cols = Vec::new();
                for r in rows {
                    let c_name: Option<&str> = r.get(0);
                    let c_type: Option<&str> = r.get(1);
                    let c_len: Option<i32> = r.get(2);
                    let c_null: Option<&str> = r.get(3);
                    cols.push(json!({
                        "column": c_name.unwrap_or(""),
                        "type": c_type.unwrap_or(""),
                        "maxLength": c_len,
                        "nullable": c_null.unwrap_or("") == "YES"
                    }));
                }
                Ok::<Value, Failure>(json!({
                    "name": name_str,
                    "kind": "relation",
                    "columns": cols
                }))
            }
        }
    })?;

    Ok(result_data)
}

pub fn query(
    project: &Project,
    args: &SqlQueryArgs,
    _context: &OperationContext,
) -> Result<Value, Failure> {
    if select_engine(project, args.engine)? == harness_domain::sql::SqlEngine::Postgresql {
        return Err((
            "PG_QUERY_UNAVAILABLE",
            "PostgreSQL currently supports status/search/describe only; arbitrary queries are not enabled",
        ));
    }
    let config = load_sql_config(project);
    if !config.enabled {
        return Err((
            "SQL_CONFIG",
            "SQL Server no está habilitado en harness.config.json",
        ));
    }

    assert_sql_read_only(&args.sql)?;

    let sql_str = args.sql.clone();
    let max_rows = config.max_rows;
    let project_clone = project.clone();
    let config_clone = config.clone();

    let output = block_on_safe(async move {
        let mut client = connect(&project_clone, &config_clone).await?;
        let stream = client.simple_query(&sql_str).await.map_err(|e| {
            eprintln!("SQL execution error: {e}");
            ("SQL_QUERY", "Error al ejecutar la consulta en SQL Server")
        })?;

        let results = stream
            .into_first_result()
            .await
            .map_err(|_| ("SQL_QUERY", "Error procesando resultados de la consulta"))?;

        let mut rows_out = Vec::new();
        for (idx, row) in results.into_iter().enumerate() {
            if idx >= max_rows {
                break;
            }
            let mut row_map = serde_json::Map::new();
            for (col_idx, col) in row.columns().iter().enumerate() {
                let col_name = col.name();
                let val_str = match row.try_get::<&str, _>(col_idx) {
                    Ok(Some(s)) => Value::String(s.to_string()),
                    Ok(None) => Value::Null,
                    Err(_) => match row.try_get::<i32, _>(col_idx) {
                        Ok(Some(n)) => json!(n),
                        Ok(None) => Value::Null,
                        Err(_) => match row.try_get::<i64, _>(col_idx) {
                            Ok(Some(n)) => json!(n),
                            Ok(None) => Value::Null,
                            Err(_) => match row.try_get::<f64, _>(col_idx) {
                                Ok(Some(f)) => json!(f),
                                Ok(None) => Value::Null,
                                Err(_) => match row.try_get::<bool, _>(col_idx) {
                                    Ok(Some(b)) => json!(b),
                                    Ok(None) => Value::Null,
                                    Err(_) => {
                                        match row.try_get::<chrono::NaiveDateTime, _>(col_idx) {
                                            Ok(Some(dt)) => Value::String(
                                                dt.format("%Y-%m-%dT%H:%M:%S").to_string(),
                                            ),
                                            Ok(None) => Value::Null,
                                            Err(_) => match row
                                                .try_get::<chrono::NaiveDate, _>(col_idx)
                                            {
                                                Ok(Some(d)) => {
                                                    Value::String(d.format("%Y-%m-%d").to_string())
                                                }
                                                Ok(None) => Value::Null,
                                                Err(_) => match row
                                                    .try_get::<rust_decimal::Decimal, _>(col_idx)
                                                {
                                                    Ok(Some(dec)) => json!(dec),
                                                    Ok(None) => Value::Null,
                                                    Err(_) => {
                                                        match row.try_get::<i16, _>(col_idx) {
                                                            Ok(Some(n)) => json!(n),
                                                            Ok(None) => Value::Null,
                                                            Err(_) => match row
                                                                .try_get::<u8, _>(col_idx)
                                                            {
                                                                Ok(Some(n)) => json!(n),
                                                                Ok(None) => Value::Null,
                                                                Err(_) => Value::String(
                                                                    "[data]".to_string(),
                                                                ),
                                                            },
                                                        }
                                                    }
                                                },
                                            },
                                        }
                                    }
                                },
                            },
                        },
                    },
                };
                row_map.insert(col_name.to_string(), val_str);
            }
            rows_out.push(Value::Object(row_map));
        }

        Ok::<Vec<Value>, Failure>(rows_out)
    })?;

    let full_json = serde_json::to_string(&output).unwrap_or_default();
    let max_chars = config.max_chars;
    let truncated = full_json.len() > max_chars;

    Ok(json!({
        "rowCount": output.len(),
        "rows": if truncated { Value::Array(output.into_iter().take(5).collect()) } else { Value::Array(output) },
        "truncated": truncated,
        "readOnlyVerified": true
    }))
}

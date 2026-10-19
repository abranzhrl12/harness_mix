//! SQL Server native adapter module powered by Tiberius.
//! Reads configuration from .agents/harness.config.json or Web.config,
//! and handles sql_status, sql_search, sql_describe, and sql_query.

use super::sql_policy::{Rows, checked};
use crate::adapters::project::Project;
use futures_util::TryStreamExt;
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
use std::time::Duration;
use tiberius::{AuthMethod, Client, Config};
use tokio::net::TcpStream;
use tokio_util::compat::TokioAsyncWriteCompatExt;

pub(super) fn block_on_safe<F, T>(future: F) -> T
where
    F: std::future::Future<Output = T> + Send,
    T: Send,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| handle.block_on(future))
        }
        Ok(_) => std::thread::scope(|scope| {
            scope
                .spawn(|| block_on_safe_standalone(future))
                .join()
                .expect("SQL worker panicked")
        }),
        Err(_) => block_on_safe_standalone(future),
    }
}

fn block_on_safe_standalone<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build standalone SQL runtime")
        .block_on(future)
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
    if let Ok(Some((_, name))) =
        super::sql_discovery::discover(project, Some(harness_domain::sql::SqlEngine::SqlServer))
    {
        config.enabled = true;
        config.connection_source = "environment".into();
        config.connection_name = Some(name);
    }
    config
}

/// Extract connection string from Web.config or environment
pub fn resolve_connection_string(project: &Project, config: &SqlConfig) -> Result<String, Failure> {
    if config.connection_source == "environment" {
        return super::sql_credentials::resolve(project, config.connection_name.as_deref().unwrap_or("HARNESS_SQL_CONNECTION_STRING"), config.env_file.as_deref(), &OperationContext::default())
            .map_err(|(code, _)| match code { "CANCELLED" | "TIMEOUT" => (code, "Credential lookup interrupted"), _ => ("SQL_CONFIG", "Set the selected SQL Server connection variable in the process environment or configured project dotenv file") });
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

    let tcp = TcpStream::connect(t_config.get_addr()).await.map_err(|_| {
        (
            "SQL_CONNECTION",
            "No se pudo conectar al servidor SQL; verifique la IP y puerto",
        )
    })?;

    let client = Client::connect(t_config, tcp.compat_write())
        .await
        .map_err(sql_error)?;

    Ok(client)
}

/// Classify errors without returning raw driver text or connection details.
fn sql_error(error: tiberius::error::Error) -> Failure {
    match error.code() {
        Some(207) => ("SQL_INVALID_COLUMN", "SQL Server 207: Invalid column name"),
        Some(208) => ("SQL_INVALID_OBJECT", "SQL Server 208: Invalid object name"),
        Some(209) => (
            "SQL_AMBIGUOUS_COLUMN",
            "SQL Server 209: Ambiguous column name",
        ),
        Some(2812) => (
            "SQL_PROCEDURE_NOT_FOUND",
            "SQL Server 2812: Stored procedure not found",
        ),
        Some(4104) => (
            "SQL_IDENTIFIER_BINDING",
            "SQL Server 4104: Multi-part identifier could not be bound",
        ),
        Some(102 | 156) => (
            "SQL_SYNTAX",
            "SQL Server syntax error; check identifiers and SQL keywords",
        ),
        Some(18456 | 18452) => ("SQL_AUTH", "SQL Server authentication failed"),
        Some(229 | 230 | 262 | 297 | 916) => {
            ("SQL_PERMISSION", "SQL Server login lacks permission")
        }
        Some(1222) => ("SQL_LOCK_TIMEOUT", "SQL Server lock wait timed out"),
        Some(1205) => (
            "SQL_DEADLOCK",
            "SQL Server selected this operation as deadlock victim",
        ),
        _ => match error {
            tiberius::error::Error::Tls(_) => (
                "SQL_TLS",
                "SQL Server TLS negotiation or certificate validation failed",
            ),
            tiberius::error::Error::Io { .. } => ("SQL_CONNECTION", "SQL Server transport failed"),
            _ => (
                "SQL_QUERY",
                "SQL Server operation failed; driver details withheld",
            ),
        },
    }
}

/// Only recognized schema errors may echo a single, bounded SQL identifier.
/// Reconstruct the message rather than trying to redact arbitrary server text.
fn safe_schema_message(number: u32, message: &str) -> Option<String> {
    let (prefix, suffix) = match number {
        207 => ("Invalid column name '", "'."),
        208 => ("Invalid object name '", "'."),
        209 => ("Ambiguous column name '", "'."),
        2812 => ("Could not find stored procedure '", "'."),
        4104 => ("The multi-part identifier \"", "\" could not be bound."),
        _ => return None,
    };
    let identifier = message.strip_prefix(prefix)?.strip_suffix(suffix)?;
    if identifier.is_empty()
        || identifier.len() > 256
        || !identifier
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '[' | ']' | '@' | '#' | '$'))
    {
        return None;
    }
    Some(format!("SQL Server {number}: {prefix}{identifier}{suffix}"))
}

fn server_diagnostic(error: &tiberius::error::Error) -> Option<Value> {
    let tiberius::error::Error::Server(server) = error else {
        return None;
    };
    let number = server.code();
    // Never expose authentication, conversion, duplicate-value or user-thrown text.
    let message = safe_schema_message(number, server.message());
    Some(json!({"number": number, "message": message, "messageSanitized": true}))
}

fn operation_failed((code, message): Failure) -> Value {
    json!({"ok":false,"engine":"sqlserver","timedOut":super::sql_policy::timed_out(code),"cancelled":code=="CANCELLED",
        "diagnostics":[{"code":code,"message":message}],"connectionReused":false,
        "connectionDisposition":"closed_if_opened","cancellationRequestSent":false,"serverCancellationConfirmed":false})
}

pub fn status(
    project: &Project,
    args: &SqlStatusArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    if select_engine(project, args.engine)? == harness_domain::sql::SqlEngine::Postgresql {
        return super::postgresql::status(project, args, context);
    }
    context.check()?;
    let config = load_sql_config(project);
    if !config.enabled {
        return Ok(super::sql_policy::not_configured("sqlserver"));
    }

    let should_connect = args.connect.unwrap_or(false);
    let mut connection_verified = false;
    let mut failure = None;

    if should_connect {
        let project_clone = project.clone();
        let config_clone = config.clone();
        let timed =
            context.with_execution_timeout(Some(Duration::from_secs(config.timeout_seconds)));
        let res = block_on_safe(checked(
            async move {
                let mut client = connect(&project_clone, &config_clone).await?;
                let row = client
                    .query("SELECT 1 AS connection_ok", &[])
                    .await
                    .map_err(sql_error)?
                    .into_row()
                    .await
                    .map_err(sql_error)?;
                if row.is_some() {
                    Ok::<(), Failure>(())
                } else {
                    Err(("SQL_QUERY", "SELECT 1 no devolvió filas"))
                }
            },
            &timed,
        ));
        connection_verified = res.is_ok();
        failure = res.err();
    }

    let mut result = json!({
        "engine": "sqlserver",
        "configured": true,
        "status": if connection_verified { "ready" } else if should_connect { "verification_failed" } else { "configured_unverified" },
        "connected": connection_verified,
        "backend": "tiberius (native Rust)",
        "defaultSchema": config.default_schema,
        "connectionSource": config.connection_source,
        "connectionName": config.connection_name,
        "maxRows": config.max_rows,
        "maxChars": config.max_chars,
        "timeoutSeconds": config.timeout_seconds,
        "readOnly": true,
        "readOnlyEnforcement": "lexical_preflight_and_login_permissions",
        "querySupported": true,
        "supportedActions": ["status", "search", "describe", "query"],
        "schemaCacheEnabled": false,
        "permissionBoundary": "SQL login permissions"
    });
    if let Some((code, message)) = failure {
        super::sql_policy::verification_failed(&mut result, code, message);
        result["connectionDisposition"] = json!("closed_if_opened");
        result["serverCancellationConfirmed"] = json!(false);
        result["setup"] = super::sql_policy::configuration_guide("sqlserver");
    }
    Ok(result)
}

pub fn search(
    project: &Project,
    args: &SqlSearchArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    if select_engine(project, args.engine)? == harness_domain::sql::SqlEngine::Postgresql {
        return super::postgresql::search(project, args, context);
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

    let limit = args.limit.unwrap_or(20).clamp(1, 50).min(config.max_rows);
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
        limit + 1,
        types_clause
    );

    let escaped_query = format!("%{}%", escape_like(query));
    let schema_param = schema.to_string();

    let project_clone = project.clone();
    let config_clone = config.clone();

    let timed = context.with_execution_timeout(Some(Duration::from_secs(config.timeout_seconds)));
    let rows_data = match block_on_safe(checked(
        async {
            let mut client = connect(&project_clone, &config_clone).await?;
            let stream = client
                .query(sql.as_str(), &[&schema_param, &escaped_query])
                .await
                .map_err(sql_error)?;

            let mut rows = stream.into_row_stream();
            let mut list = Rows::new(limit, config.max_chars);
            while let Some(r) = rows.try_next().await.map_err(sql_error)? {
                let schema: Option<&str> = r.get(0);
                let name: Option<&str> = r.get(1);
                let kind: Option<&str> = r.get(2);
                if !list.push(
                    json!({
                        "schema": schema.unwrap_or(""),
                        "name": name.unwrap_or(""),
                        "kind": kind.unwrap_or("")
                    }),
                    &timed,
                )? {
                    break;
                }
            }
            Ok::<Rows, Failure>(list)
        },
        &timed,
    )) {
        Ok(value) => value,
        Err(error) => return Ok(operation_failed(error)),
    };

    Ok(json!({
        "engine": "sqlserver",
        "count": rows_data.values.len(),
        "results": rows_data.values,
        "truncated": rows_data.truncated,
        "query": query,
        "schema": schema
    }))
}

pub fn describe(
    project: &Project,
    args: &SqlDescribeArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    if select_engine(project, args.engine)? == harness_domain::sql::SqlEngine::Postgresql {
        return super::postgresql::describe(project, args, context);
    }
    if args.refresh == Some(true) {
        return Err((
            "SQL_CACHE_UNAVAILABLE",
            "refresh is available only for PostgreSQL schema snapshots; SQL Server describe always reads live",
        ));
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
    if matches!(kind, SqlDescribeKind::Routine)
        && args
            .sections
            .as_ref()
            .is_some_and(|sections| !sections.is_empty())
    {
        return Err((
            "SQL_DESCRIBE_SECTION",
            "SQL Server routine describe does not support sections",
        ));
    }
    if args
        .sections
        .as_ref()
        .is_some_and(|sections| sections.iter().any(|s| s != "columns"))
    {
        return Err((
            "SQL_DESCRIBE_SECTION",
            "SQL Server relation describe supports columns only",
        ));
    }
    let name_str = args.name.clone();
    let project_clone = project.clone();
    let config_clone = config.clone();

    let timed = context.with_execution_timeout(Some(Duration::from_secs(config.timeout_seconds)));
    let result_data = match block_on_safe(checked(
        async {
            let mut client = connect(&project_clone, &config_clone).await?;
            match kind {
                SqlDescribeKind::Routine => {
                    let sql = "SELECT OBJECT_DEFINITION(OBJECT_ID(@P1)) AS def";
                    let stream = client.query(sql, &[&name_str]).await.map_err(sql_error)?;
                    let row = stream.into_row().await.map_err(sql_error)?;
                    let def: Option<&str> = row.as_ref().and_then(|r| r.try_get(0).ok().flatten());
                    let mut bounded = Rows::new(1, config.max_chars);
                    bounded.push(
                        json!({"definition":def.unwrap_or("No definition available")}),
                        &timed,
                    )?;
                    Ok::<Value, Failure>(json!({
                        "engine": "sqlserver",
                        "name": name_str,
                        "kind": "routine",
                        "definition": bounded.values.first().map(|v| v["definition"].clone()).unwrap_or(Value::Null),
                        "truncated": bounded.truncated
                    }))
                }
                SqlDescribeKind::Relation => {
                    let (schema_name, relation_name) = name_str
                        .split_once('.')
                        .unwrap_or((&config.default_schema, &name_str));
                    let sql = "SELECT COLUMN_NAME, DATA_TYPE, CHARACTER_MAXIMUM_LENGTH, IS_NULLABLE \
                               FROM INFORMATION_SCHEMA.COLUMNS \
                               WHERE TABLE_NAME = @P1 AND TABLE_SCHEMA = @P2 \
                               ORDER BY ORDINAL_POSITION";
                    let stream = client
                        .query(sql, &[&relation_name, &schema_name])
                        .await
                        .map_err(sql_error)?;
                    let mut rows = stream.into_row_stream();
                    let mut cols = Rows::new(config.max_rows, config.max_chars);
                    while let Some(r) = rows.try_next().await.map_err(sql_error)? {
                        let c_name: Option<&str> = r.get(0);
                        let c_type: Option<&str> = r.get(1);
                        let c_len: Option<i32> = r.get(2);
                        let c_null: Option<&str> = r.get(3);
                        if !cols.push(
                            json!({
                                "column": c_name.unwrap_or(""),
                                "type": c_type.unwrap_or(""),
                                "maxLength": c_len,
                                "nullable": c_null.unwrap_or("") == "YES"
                            }),
                            &timed,
                        )? {
                            break;
                        }
                    }
                    Ok::<Value, Failure>(json!({
                        "engine": "sqlserver",
                        "name": name_str,
                        "kind": "relation",
                        "columns": cols.values,
                        "truncated": cols.truncated
                    }))
                }
            }
        },
        &timed,
    )) {
        Ok(value) => value,
        Err(error) => return Ok(operation_failed(error)),
    };

    Ok(result_data)
}

pub fn query(
    project: &Project,
    args: &SqlQueryArgs,
    context: &OperationContext,
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

    let (sql_str, parameters) = parameters::prepare(&args.sql, args.parameters.as_ref())?;
    let server_error = std::sync::Mutex::new(None);
    let report_error = |error: tiberius::error::Error| {
        if let Ok(mut diagnostic) = server_error.lock() {
            *diagnostic = server_diagnostic(&error);
        }
        sql_error(error)
    };
    let max_rows = config.max_rows;
    let project_clone = project.clone();
    let config_clone = config.clone();

    let timed = context.with_execution_timeout(Some(Duration::from_secs(config.timeout_seconds)));
    let output = match block_on_safe(checked(
        async {
            let mut client = connect(&project_clone, &config_clone).await?;
            let mut query = tiberius::Query::new(sql_str);
            for value in parameters {
                parameters::bind(&mut query, value);
            }
            let stream = query.query(&mut client).await.map_err(&report_error)?;

            let mut results = stream.into_row_stream();
            let mut rows_out = Rows::new(max_rows, config.max_chars);
            while let Some(row) = results.try_next().await.map_err(&report_error)? {
                if row.result_index() != 0 {
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
                                                    Ok(Some(d)) => Value::String(
                                                        d.format("%Y-%m-%d").to_string(),
                                                    ),
                                                    Ok(None) => Value::Null,
                                                    Err(_) => match row
                                                        .try_get::<rust_decimal::Decimal, _>(
                                                            col_idx,
                                                        ) {
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
                if !rows_out.push(Value::Object(row_map), &timed)? {
                    break;
                }
            }

            Ok::<Rows, Failure>(rows_out)
        },
        &timed,
    )) {
        Ok(value) => value,
        Err(error) => {
            let mut output = operation_failed(error);
            if let Ok(diagnostic) = server_error.lock() {
                if let Some(diagnostic) = diagnostic.as_ref() {
                    if let Some(message) = diagnostic["message"].as_str() {
                        output["diagnostics"][0]["message"] = json!(message);
                    }
                    output["serverError"] = diagnostic.clone();
                }
            }
            return Ok(output);
        }
    };

    Ok(json!({
        "engine": "sqlserver",
        "rowCount": output.values.len(),
        "rows": output.values,
        "truncated": output.truncated,
        "readOnlyVerified": false,
        "readOnlyPreflightPassed": true,
        "readOnlyEnforcement": "lexical_preflight_and_login_permissions",
        "connectionDiscarded": true,
        "serverCancellationConfirmed": false
    }))
}

#[path = "sql_parameters.rs"]
mod parameters;

fn escape_like(query: &str) -> String {
    let mut output = String::with_capacity(query.len());
    for c in query.chars() {
        if matches!(c, '~' | '%' | '_' | '[') {
            output.push('~');
        }
        output.push(c);
    }
    output
}

#[cfg(test)]
mod safety_tests {
    use super::*;
    #[test]
    fn literal_like_wildcards_are_escaped_without_replacement_templates() {
        assert_eq!(escape_like("a%_~[é"), "a~%~_~~~[é");
    }
    #[test]
    fn driver_error_text_is_never_exposed() {
        assert_eq!(
            sql_error(tiberius::error::Error::Tls(
                "private connection details".into()
            ))
            .0,
            "SQL_TLS"
        );
        let failure = sql_error(tiberius::error::Error::Protocol(
            "SELECT private value".into(),
        ));
        assert!(!failure.1.contains("private"));
        let output = operation_failed(("TIMEOUT", "operation deadline exceeded"));
        assert_eq!(output["timedOut"], true);
        assert_eq!(output["serverCancellationConfirmed"], false);
    }
    #[test]
    fn schema_errors_echo_only_allowlisted_identifiers() {
        assert_eq!(
            safe_schema_message(207, "Invalid column name 'tEmpresaRuc'.").as_deref(),
            Some("SQL Server 207: Invalid column name 'tEmpresaRuc'.")
        );
        assert!(safe_schema_message(208, "Invalid object name 'dbo.MEMPRESA'.").is_some());
        assert!(
            safe_schema_message(
                4104,
                "The multi-part identifier \"a.Id\" could not be bound."
            )
            .is_some()
        );
        for message in [
            "Invalid column name 'Password=secret'.",
            "Invalid column name 'valid'.\nUser ID=sa;Password=secret",
            "Invalid column name 'abc\nsecret'.",
            "Invalid column name 'SELECT private value'.",
            "Invalid column name 'x';Password=secret'.",
            "Invalid column name ''.",
        ] {
            assert!(safe_schema_message(207, message).is_none());
        }
        assert!(
            safe_schema_message(207, &format!("Invalid column name '{}'.", "x".repeat(257)))
                .is_none()
        );
        assert!(safe_schema_message(18456, "Login failed for user 'secret'.").is_none());
        assert!(
            safe_schema_message(245, "Conversion failed when converting 'private value'.")
                .is_none()
        );
        assert!(safe_schema_message(50000, "Invalid column name 'secret'.").is_none());
        assert!(
            server_diagnostic(&tiberius::error::Error::Protocol("Password=secret".into()))
                .is_none()
        );
    }
    #[tokio::test]
    async fn sync_bridge_handles_current_thread_runtime() {
        assert_eq!(
            block_on_safe(async {
                tokio::time::sleep(Duration::from_millis(1)).await;
                42
            }),
            42
        );
    }
}

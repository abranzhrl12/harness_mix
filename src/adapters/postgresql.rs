//! PostgreSQL catalog access. Only fixed, parameterized metadata queries are exposed.
use super::project::Project;
#[path = "postgresql_pool.rs"]
mod pool;
use futures_util::TryStreamExt;
use harness_domain::{
    Failure,
    execution::OperationContext,
    sql::{SqlDescribeArgs, SqlDescribeKind, SqlObjectType, SqlSearchArgs, SqlStatusArgs},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{future::Future, time::Duration};
use tokio_postgres::{
    Config as DriverConfig,
    config::{Host, SslMode},
    types::ToSql,
};

const CONFIG: Failure = (
    "PG_CONFIG",
    "Invalid postgresql configuration; check limits, schemas, TLS and environment connection source",
);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Config {
    #[serde(skip)]
    fingerprint: String,
    #[serde(default)]
    enabled: bool,
    #[serde(default = "default_schema")]
    default_schema: String,
    #[serde(default = "default_schemas")]
    allowed_schemas: Vec<String>,
    #[serde(default = "default_rows")]
    max_rows: usize,
    #[serde(default = "default_chars")]
    max_chars: usize,
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
    #[serde(default)]
    schema_cache_seconds: Option<u64>,
    connection: Option<Connection>,
    #[serde(default)]
    tls: Tls,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Connection {
    source: String,
    name: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Tls {
    mode: String,
    ca_file: Option<String>,
}
impl Default for Tls {
    fn default() -> Self {
        Self {
            mode: "verify-full".into(),
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

fn load(project: &Project) -> Result<Option<Config>, Failure> {
    let path = project.target_optional(".agents/harness.config.json")?;
    let bytes = match std::fs::read(&path) {
        Ok(bytes) if bytes.len() <= 1024 * 1024 => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        _ => return Err(CONFIG),
    };
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| CONFIG)?;
    let Some(pg) = value.get("postgresql") else {
        return Ok(None);
    };
    let mut config: Config = serde_json::from_value(pg.clone()).map_err(|_| CONFIG)?;
    config.fingerprint = harness_domain::hash(&bytes);
    if !(1..=200).contains(&config.max_rows)
        || !(1000..=32000).contains(&config.max_chars)
        || !(1..=120).contains(&config.timeout_seconds)
        || config.allowed_schemas.is_empty()
        || config.allowed_schemas.len() > 16
        || !config.allowed_schemas.contains(&config.default_schema)
        || config
            .allowed_schemas
            .iter()
            .any(|s| s.is_empty() || s.len() > 63 || s.contains('\0'))
        || !["verify-full", "disable-loopback"].contains(&config.tls.mode.as_str())
    {
        return Err(CONFIG);
    }
    Ok(Some(config))
}
pub(super) fn enabled(project: &Project) -> Result<bool, Failure> {
    Ok(load(project)?.is_some_and(|c| c.enabled))
}
fn required(project: &Project) -> Result<Config, Failure> {
    load(project)?
        .filter(|c| c.enabled)
        .ok_or(("PG_CONFIG", "PostgreSQL is not enabled"))
}

fn driver_config(config: &Config) -> Result<(DriverConfig, String), Failure> {
    let connection = config.connection.as_ref().ok_or(CONFIG)?;
    if connection.source != "environment"
        || connection.name.is_empty()
        || !connection
            .name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(CONFIG);
    }
    let dsn = std::env::var(&connection.name).map_err(|_| {
        (
            "PG_CREDENTIALS",
            "PostgreSQL connection environment variable is unavailable",
        )
    })?;
    let mut driver: DriverConfig = dsn.parse().map_err(|_| {
        (
            "PG_CONNECTION_CONFIG",
            "Invalid PostgreSQL connection string",
        )
    })?;
    if driver.get_hosts().is_empty() || !driver.get_hostaddrs().is_empty() {
        return Err(CONFIG);
    }
    if config.tls.mode == "disable-loopback" {
        if config.tls.ca_file.is_some()
            || driver.get_hosts().iter().any(|h| match h {
                Host::Tcp(host) => {
                    host != "localhost"
                        && !host
                            .parse::<std::net::IpAddr>()
                            .is_ok_and(|ip| ip.is_loopback())
                }
                #[cfg(unix)]
                Host::Unix(_) => true,
            })
        {
            return Err((
                "PG_TLS",
                "Unencrypted PostgreSQL is allowed only with explicit loopback hosts",
            ));
        }
        driver.ssl_mode(SslMode::Disable);
    } else {
        driver.ssl_mode(SslMode::Require);
    }
    driver.connect_timeout(Duration::from_secs(config.timeout_seconds));
    if driver.get_application_name().is_none() {
        driver.application_name(format!("agent-harness:{}", std::process::id()));
    }
    Ok((driver, harness_domain::hash(dsn.as_bytes())))
}
fn certificate(project: &Project, config: &Config) -> Result<Option<Vec<u8>>, Failure> {
    if let Some(file) = &config.tls.ca_file {
        // Certificates must be workspace-relative; do not open arbitrary credential paths.
        let path = project.target(file)?;
        let metadata = std::fs::metadata(&path).map_err(|_| CONFIG)?;
        if !metadata.is_file() || metadata.len() > 256 * 1024 {
            return Err(CONFIG);
        }
        let pem = std::fs::read(path).map_err(|_| CONFIG)?;
        return Ok(Some(pem));
    }
    Ok(None)
}
fn tls(certificate: Option<&[u8]>) -> Result<postgres_native_tls::MakeTlsConnector, Failure> {
    let mut builder = native_tls::TlsConnector::builder();
    if let Some(pem) = certificate {
        builder.add_root_certificate(native_tls::Certificate::from_pem(pem).map_err(|_| CONFIG)?);
    }
    Ok(postgres_native_tls::MakeTlsConnector::new(
        builder
            .build()
            .map_err(|_| ("PG_TLS", "Unable to initialize certificate validation"))?,
    ))
}

fn pg_error(error: tokio_postgres::Error) -> Failure {
    match error.code().map(|c| c.code()) {
        Some("28P01" | "28000") => ("PG_AUTH", "PostgreSQL authentication failed"),
        Some("42501") => (
            "PG_PERMISSION",
            "PostgreSQL role cannot access the requested metadata",
        ),
        Some("57014") => (
            "PG_STATEMENT_TIMEOUT",
            "PostgreSQL statement cancelled or server timeout reached",
        ),
        Some("55P03") => ("PG_LOCK_TIMEOUT", "PostgreSQL lock wait exceeded its limit"),
        _ => (
            "PG_OPERATION",
            "PostgreSQL connection, TLS or catalog operation failed; raw server errors are withheld",
        ),
    }
}

async fn checked<F, T>(future: F, context: &OperationContext) -> Result<T, Failure>
where
    F: Future<Output = Result<T, Failure>>,
{
    tokio::pin!(future);
    loop {
        context.check()?;
        tokio::select! {
            result = &mut future => return result,
            _ = tokio::time::sleep(Duration::from_millis(25)) => {}
        }
    }
}

async fn catalog(
    project: &Project,
    config: &Config,
    context: &OperationContext,
    sql: &'static str,
    params: &[&(dyn ToSql + Sync)],
    limit: usize,
) -> Result<Value, Failure> {
    let (driver, credential_identity) = driver_config(config)?;
    let certificate = certificate(project, config)?;
    let context = context.with_execution_timeout(Some(Duration::from_secs(config.timeout_seconds)));
    // Rotate on workspace, credentials, configuration or CA content changes. No secrets are retained as keys.
    let identity = json!([
        project.root,
        config.fingerprint,
        credential_identity,
        certificate.as_deref().map(harness_domain::hash)
    ])
    .to_string();
    let mut lease = pool::checkout(harness_domain::hash(identity.as_bytes()), &context).await?;
    let reused = lease.reused;
    if lease.connection.is_none() {
        let connector = tls(certificate.as_deref())?;
        let (client, connection) = checked(
            async { driver.connect(connector.clone()).await.map_err(pg_error) },
            &context,
        )
        .await?;
        lease.connection = Some(pool::Connection::new(
            client,
            tokio::spawn(connection),
            connector,
        ));
    }
    let entry = lease.connection.as_mut().expect("connected lease");
    let cancel = entry.client.cancel_token();
    let connector = entry.connector.clone();
    let result = checked(async {
        let transaction = entry.client.build_transaction().read_only(true).start().await.map_err(pg_error)?;
        transaction.batch_execute(&format!("SET LOCAL statement_timeout = {}; SET LOCAL lock_timeout = {}; SET LOCAL search_path = pg_catalog;",
            config.timeout_seconds * 1000, (config.timeout_seconds * 1000).min(2000))).await.map_err(pg_error)?;
        let (rows, truncated, statement_reused) = {
          let cached = entry.statements.get(sql).cloned();
          let statement_reused = cached.is_some();
          let statement = match cached {
              Some(statement) => statement,
              None => {
                  let statement = transaction.prepare(sql).await.map_err(pg_error)?;
                  if entry.statements.len() == 8 { entry.statements.clear(); }
                  entry.statements.insert(sql, statement.clone());
                  statement
              }
          };
          let stream = transaction.query_raw(&statement, params.iter().copied()).await.map_err(pg_error)?;
          tokio::pin!(stream);
          let mut rows = Vec::new();
          let mut used = 0;
          let mut truncated = false;
          while let Some(row) = stream.try_next().await.map_err(pg_error)? {
            context.check()?;
            if rows.len() == limit { truncated = true; break }
            let mut value = serde_json::Map::new();
            for (index, column) in row.columns().iter().enumerate() {
                let cell: Option<String> = row.try_get(index).map_err(|_| ("PG_RESULT", "Unexpected catalog column type"))?;
                value.insert(column.name().into(), cell.map(Value::String).unwrap_or(Value::Null));
            }
            let mut value = Value::Object(value);
            super::data_query::redact(&mut value, 0, &mut 0, &context)?;
            let size = value.to_string().len();
            if used + size > config.max_chars { truncated = true; break }
            used += size;
            rows.push(value);
          }
          (rows, truncated, statement_reused)
        };
        transaction.rollback().await.map_err(pg_error)?;
        Ok(json!({"engine":"postgresql","results":rows,"truncated":truncated,
            "count":rows.len(),"readOnly":true,"visibility":"connected_role_and_allowed_schemas",
            "connectionReused":reused,"statementReused":statement_reused,"resultCache":false,
            "globalFreshnessCertified":false}))
    }, &context).await;
    if result.is_err() {
        let (code, message) = *result.as_ref().err().expect("checked error");
        let cancellation =
            tokio::time::timeout(Duration::from_secs(2), cancel.cancel_query(connector)).await;
        entry.worker.abort();
        let _ = (&mut entry.worker).await;
        let sent = matches!(cancellation, Ok(Ok(())));
        let mut diagnostics = vec![json!({"code":code,"message":message})];
        if !sent {
            diagnostics.push(json!({"code":"PG_CANCEL_UNCONFIRMED","message":"Connection closed; cancellation request delivery could not be confirmed"}));
        }
        // A successful CancelRequest write does not acknowledge server completion.
        return Ok(
            json!({"ok":false,"timedOut":code=="TIMEOUT","cancelled":code=="CANCELLED",
            "diagnostics":diagnostics,"engine":"postgresql","connectionReused":reused,"connectionDiscarded":true,
            "cancellationRequestSent":sent,"serverCancellationConfirmed":false}),
        );
    }
    // Only confirmed rollback makes the lease eligible for reuse. All other exits discard it.
    lease.reusable = true;
    result
}

pub(super) fn status(
    project: &Project,
    args: &SqlStatusArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    let Some(config) = load(project)?.filter(|c| c.enabled) else {
        return Ok(
            json!({"engine":"postgresql","configured":false,"connected":false,"status":"not_configured"}),
        );
    };
    let mut result = json!({"engine":"postgresql","configured":true,"connected":false,"status":"configured_unverified",
        "defaultSchema":config.default_schema,"allowedSchemas":config.allowed_schemas,"maxRows":config.max_rows,
        "maxChars":config.max_chars,"timeoutSeconds":config.timeout_seconds,"tlsMode":config.tls.mode,
        "supportedActions":["status","search","describe"],"querySupported":false,"schemaCacheEnabled":false,
        "schemaCacheSecondsRequested":config.schema_cache_seconds,"readOnly":true,
        "connectionPooling":{"enabled":true,"maxConnections":pool::CAPACITY,"idleSeconds":60,"maxLifetimeSeconds":300}});
    if args.connect == Some(true) {
        match pool::run(catalog(
            project,
            &config,
            context,
            "SELECT current_database()::text AS database, current_user::text AS role, current_setting('server_version')::text AS version",
            &[],
            1,
        )) {
            Ok(probe) if probe["ok"] != false => {
                result["connected"] = json!(true);
                result["status"] = json!("ready");
                result["server"] = probe["results"].clone();
                result["connectionReused"] = probe["connectionReused"].clone();
            }
            Ok(probe) => return Ok(probe),
            Err((code, message)) => {
                result["ok"] = json!(false);
                result["timedOut"] = json!(code == "TIMEOUT");
                result["cancelled"] = json!(code == "CANCELLED");
                result["status"] = json!("verification_failed");
                result["diagnostics"] = json!([{"code":code,"message":message}]);
            }
        }
    }
    Ok(result)
}

fn schema<'a>(config: &'a Config, requested: Option<&'a str>) -> Result<&'a str, Failure> {
    let schema = requested.unwrap_or(&config.default_schema);
    if !config.allowed_schemas.iter().any(|s| s == schema) {
        return Err(("PG_SCHEMA", "Schema is outside allowedSchemas"));
    }
    Ok(schema)
}

const RELATIONS: &str = "SELECT n.nspname::text AS schema,c.relname::text AS name,CASE c.relkind WHEN 'v' THEN 'view' WHEN 'm' THEN 'view' ELSE 'table' END::text AS kind FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relkind IN ('r','p','v','m') AND pg_catalog.has_schema_privilege(n.oid,'USAGE') AND pg_catalog.has_table_privilege(c.oid,'SELECT') AND ($3='' OR CASE WHEN c.relkind IN ('v','m') THEN 'view' ELSE 'table' END=$3) AND strpos(lower(c.relname),lower($2))>0 ORDER BY c.relname LIMIT $4";
const COLUMNS: &str = "SELECT n.nspname::text AS schema,c.relname::text AS name,a.attname::text AS column,'column'::text AS kind,pg_catalog.format_type(a.atttypid,a.atttypmod)::text AS data_type FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid=a.attrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND a.attnum>0 AND NOT a.attisdropped AND c.relkind IN ('r','p','v','m') AND pg_catalog.has_schema_privilege(n.oid,'USAGE') AND pg_catalog.has_column_privilege(c.oid,a.attnum,'SELECT') AND strpos(lower(a.attname),lower($2))>0 AND $3='' ORDER BY c.relname,a.attnum LIMIT $4";
const ROUTINES: &str = "SELECT n.nspname::text AS schema,p.proname::text AS name,CASE p.prokind WHEN 'p' THEN 'procedure' ELSE 'function' END::text AS kind,left(pg_catalog.pg_get_function_identity_arguments(p.oid),1000)::text AS identity_arguments FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname=$1 AND p.prokind IN ('f','p') AND pg_catalog.has_schema_privilege(n.oid,'USAGE') AND pg_catalog.has_function_privilege(p.oid,'EXECUTE') AND strpos(lower(p.proname),lower($2))>0 AND ($3='' OR CASE p.prokind WHEN 'p' THEN 'procedure' ELSE 'function' END=$3) ORDER BY p.proname,p.oid LIMIT $4";

pub(super) fn search(
    project: &Project,
    args: &SqlSearchArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    let config = required(project)?;
    if args.query.is_empty() || args.query.len() > 128 {
        return Err(("ARGUMENT", "Catalog query must contain 1..128 bytes"));
    }
    let selected_schema = schema(&config, args.schema.as_deref())?;
    let limit = args.limit.unwrap_or(20).min(config.max_rows);
    if limit == 0 {
        return Err(("ARGUMENT", "limit must be positive"));
    }
    let (sql, kind) = match args.object_type {
        Some(SqlObjectType::Column) => (COLUMNS, ""),
        Some(SqlObjectType::Procedure) => (ROUTINES, "procedure"),
        Some(SqlObjectType::Function) => (ROUTINES, "function"),
        Some(SqlObjectType::View) => (RELATIONS, "view"),
        Some(SqlObjectType::Table) | None => (RELATIONS, ""),
    };
    let kind = if matches!(args.object_type, Some(SqlObjectType::Table)) {
        "table"
    } else {
        kind
    };
    let count = (limit + 1) as i64;
    let mut result = pool::run(catalog(
        project,
        &config,
        context,
        sql,
        &[&selected_schema, &args.query, &kind, &count],
        limit,
    ))?;
    result["objectTypeDefault"] =
        json!("relations; specify function/procedure/column for other catalogs");
    Ok(result)
}

const DESCRIBE: &str = "SELECT n.nspname::text AS schema,c.relname::text AS name,a.attname::text AS column,pg_catalog.format_type(a.atttypid,a.atttypmod)::text AS data_type,(NOT a.attnotnull)::text AS nullable FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid=a.attrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2 AND c.relkind IN ('r','p','v','m') AND a.attnum>0 AND NOT a.attisdropped AND pg_catalog.has_schema_privilege(n.oid,'USAGE') AND pg_catalog.has_column_privilege(c.oid,a.attnum,'SELECT') ORDER BY a.attnum LIMIT $3";
pub(super) fn describe(
    project: &Project,
    args: &SqlDescribeArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    let config = required(project)?;
    if args.name.len() > 256 || args.name.contains('\0') {
        return Err(("ARGUMENT", "Invalid exact PostgreSQL object name"));
    }
    if matches!(args.kind, Some(SqlDescribeKind::Routine)) {
        return Err((
            "PG_DESCRIBE_UNAVAILABLE",
            "Routine signatures are available through search; routine definitions are not exposed yet",
        ));
    }
    if args
        .sections
        .as_ref()
        .is_some_and(|s| s.iter().any(|s| s != "columns"))
    {
        return Err((
            "PG_DESCRIBE_SECTION",
            "This PostgreSQL version supports the columns section only",
        ));
    }
    let (selected_schema, name) = args
        .name
        .split_once('.')
        .unwrap_or((&config.default_schema, &args.name));
    let selected_schema = schema(&config, Some(selected_schema))?;
    let count = (config.max_rows + 1) as i64;
    let mut result = pool::run(catalog(
        project,
        &config,
        context,
        DESCRIBE,
        &[&selected_schema, &name, &count],
        config.max_rows,
    ))?;
    result["sections"] = json!(["columns"]);
    result["omittedSections"] = json!([
        "defaults",
        "constraints",
        "indexes",
        "comments",
        "viewDefinition",
        "routineDefinition"
    ]);
    result["absenceIsConclusive"] = json!(false);
    Ok(result)
}

use super::{
    block_on_safe,
    config::load_sql_config,
    connection::connect,
    diagnostics::{
        operation_failed, operation_failed_with_server_diagnostic, server_diagnostic, sql_error,
    },
    engine::select_engine,
    parameters,
};
use crate::adapters::project::Project;
use crate::adapters::sql_policy::{Rows, checked};
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
use std::time::Duration;

pub(super) fn status(
    project: &Project,
    args: &SqlStatusArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    if select_engine(project, args.engine)? == harness_domain::sql::SqlEngine::Postgresql {
        return crate::adapters::postgresql::status(project, args, context);
    }
    context.check()?;
    let config = load_sql_config(project);
    if !config.enabled {
        return Ok(crate::adapters::sql_policy::not_configured("sqlserver"));
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
        crate::adapters::sql_policy::verification_failed(&mut result, code, message);
        result["connectionDisposition"] = json!("closed_if_opened");
        result["serverCancellationConfirmed"] = json!(false);
        result["setup"] = crate::adapters::sql_policy::configuration_guide("sqlserver");
    }
    Ok(result)
}

pub(super) fn search(
    project: &Project,
    args: &SqlSearchArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    if select_engine(project, args.engine)? == harness_domain::sql::SqlEngine::Postgresql {
        return crate::adapters::postgresql::search(project, args, context);
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
    let schema = args.schema.as_deref().unwrap_or("*");
    let schema = if schema == "*" { "" } else { schema };

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
        "schema": if schema.is_empty() { "*" } else { schema }
    }))
}

pub(super) fn describe(
    project: &Project,
    args: &SqlDescribeArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    if select_engine(project, args.engine)? == harness_domain::sql::SqlEngine::Postgresql {
        return crate::adapters::postgresql::describe(project, args, context);
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

/// Injects `TOP (limit)` into a top-level SELECT without one, so large tables
/// cannot stream unbounded rows before the row budget truncates output.
pub(super) fn inject_top(sql: &str, limit: usize) -> String {
    let bytes = sql.as_bytes();
    let len = bytes.len();
    let from = |mut i: usize| -> usize {
        loop {
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i + 1 < len && bytes[i] == b'-' && bytes[i + 1] == b'-' {
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                i += 2;
                while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(len);
                continue;
            }
            return i;
        }
    };
    let start = from(0);
    if len < start + 6 || !sql[start..start + 6].eq_ignore_ascii_case("SELECT") {
        // WITH-CTEs and anything else are left untouched.
        return sql.to_owned();
    }
    let mut end = start + 6;
    loop {
        let next = from(end);
        if next >= len {
            break;
        }
        let word_end = next
            + sql[next..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .count();
        match sql[next..word_end].to_ascii_uppercase().as_str() {
            "ALL" | "DISTINCT" => end = word_end,
            "TOP" => return sql.to_owned(),
            _ => break,
        }
    }
    format!("{} TOP ({limit}){}", &sql[..end], &sql[end..])
}

pub(super) fn query(
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
    let sql_str = inject_top(&sql_str, config.max_rows);
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
            let diagnostic = server_error.lock().ok();
            return Ok(operation_failed_with_server_diagnostic(
                error,
                diagnostic.as_deref().and_then(Option::as_ref),
            ));
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

pub(super) fn escape_like(query: &str) -> String {
    let mut output = String::with_capacity(query.len());
    for c in query.chars() {
        if matches!(c, '~' | '%' | '_' | '[') {
            output.push('~');
        }
        output.push(c);
    }
    output
}

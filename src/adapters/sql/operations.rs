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
        SqlDescribeArgs, SqlDescribeKind, SqlMapArgs, SqlObjectType, SqlQueryArgs, SqlSearchArgs,
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
        "supportedActions": ["status", "search", "map", "describe", "query"],
        "workflow": {"overview": "map", "discover": "search", "inspect": "describe", "parallelTool": "harness_batch", "maxJobs": 4, "maxConcurrent": 2},
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

/// One round trip that answers the whole schema map: relations with their
/// Core schema map. It only reads catalog views that a least-privilege login
/// with object visibility can read, so it never fails for missing statistics
/// permissions. The row estimate is a separate, optional statement.
const MAP: &str = r#"
SELECT TOP (@P2) kind, [schema], name, detail1, detail2, detail3, detail4, num1
FROM (
  SELECT 0 AS ord, CAST('relation' AS nvarchar(32)) AS kind, s.name AS [schema], o.name AS name,
         CAST(CASE o.type WHEN 'U' THEN 'table' ELSE 'view' END AS nvarchar(32)) AS detail1,
         CAST('' AS nvarchar(512)) AS detail2, CAST('' AS nvarchar(512)) AS detail3,
         CAST('' AS nvarchar(512)) AS detail4, 0 AS num1
  FROM sys.objects AS o
  JOIN sys.schemas AS s ON s.schema_id = o.schema_id
  WHERE o.is_ms_shipped = 0 AND o.type IN ('U','V') AND (s.name = @P1 OR @P1 = '')
  UNION ALL
  SELECT 1, CAST('primaryKey' AS nvarchar(32)), s.name, t.name, c.name,
         CAST('' AS nvarchar(512)), CAST('' AS nvarchar(512)), CAST('' AS nvarchar(512)), 0
  FROM sys.indexes AS i
  JOIN sys.index_columns AS ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id
  JOIN sys.columns AS c ON c.object_id = ic.object_id AND c.column_id = ic.column_id
  JOIN sys.objects AS t ON t.object_id = i.object_id
  JOIN sys.schemas AS s ON s.schema_id = t.schema_id
  WHERE i.is_primary_key = 1 AND i.is_hypothetical = 0 AND (s.name = @P1 OR @P1 = '')
  UNION ALL
  SELECT 2, CAST('foreignKey' AS nvarchar(32)), s.name, fk.name, child.name, cc.name, parent.name, pc.name, 0
  FROM sys.foreign_keys AS fk
  JOIN sys.foreign_key_columns AS fkc ON fkc.constraint_object_id = fk.object_id
  JOIN sys.tables AS child ON child.object_id = fkc.parent_object_id
  JOIN sys.columns AS cc ON cc.object_id = fkc.parent_object_id AND cc.column_id = fkc.parent_column_id
  JOIN sys.tables AS parent ON parent.object_id = fkc.referenced_object_id
  JOIN sys.columns AS pc ON pc.object_id = fkc.referenced_object_id AND pc.column_id = fkc.referenced_column_id
  JOIN sys.schemas AS s ON s.schema_id = parent.schema_id
  WHERE (s.name = @P1 OR @P1 = '')
  UNION ALL
  SELECT 3, CAST('routine' AS nvarchar(32)), s.name, o.name, p.name, t.name,
         CAST(CASE WHEN p.is_output = 1 THEN 'out' ELSE 'in' END AS nvarchar(8)),
         CAST(CASE o.type WHEN 'P' THEN 'procedure' ELSE 'function' END AS nvarchar(32)), 0
  FROM sys.objects AS o
  JOIN sys.schemas AS s ON s.schema_id = o.schema_id
  LEFT JOIN sys.parameters AS p ON p.object_id = o.object_id
  LEFT JOIN sys.types AS t ON t.user_type_id = p.user_type_id
  WHERE o.is_ms_shipped = 0 AND o.type IN ('P','FN','IF','TF') AND (s.name = @P1 OR @P1 = '')
) AS m
ORDER BY m.ord, m.[schema], m.name
"#;

/// Optional row estimates. Reading partition statistics can require elevated
/// permissions, so a failure here degrades the map instead of breaking it.
const MAP_SIZE: &str = r#"
SELECT o.name AS name, COALESCE(SUM(st.row_count), 0) AS rows_approx
FROM sys.objects AS o
JOIN sys.schemas AS s ON s.schema_id = o.schema_id
LEFT JOIN sys.dm_db_partition_stats AS st ON st.object_id = o.object_id AND st.index_id IN (0, 1)
WHERE o.is_ms_shipped = 0 AND o.type IN ('U','V') AND (s.name = @P1 OR @P1 = '')
GROUP BY o.name
"#;

pub(super) fn map(
    project: &Project,
    args: &SqlMapArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    if select_engine(project, args.engine)? == harness_domain::sql::SqlEngine::Postgresql {
        return crate::adapters::postgresql::map(project, args, context);
    }
    context.check()?;
    let config = load_sql_config(project);
    if !config.enabled {
        return Err((
            "SQL_CONFIG",
            "SQL Server no está habilitado en harness.config.json",
        ));
    }
    let schema = args.schema.as_deref().unwrap_or("*");
    let schema = if schema == "*" { "" } else { schema };
    let limit = args.limit.unwrap_or(50).clamp(1, 200);
    let row_budget = (limit * 8).min(1600);
    let schema_param = schema.to_string();
    let top_param = row_budget as i32;

    let project_clone = project.clone();
    let config_clone = config.clone();
    let timed = context.with_execution_timeout(Some(Duration::from_secs(config.timeout_seconds)));
    let (rows_data, row_estimates) = match block_on_safe(checked(
        async {
            let mut client = connect(&project_clone, &config_clone).await?;
            let mut collected: Vec<crate::adapters::sql_map::MapRow> = Vec::new();
            {
                let stream = client
                    .query(MAP, &[&schema_param, &top_param])
                    .await
                    .map_err(sql_error)?;
                let mut rows = stream.into_row_stream();
                while let Some(r) = rows.try_next().await.map_err(sql_error)? {
                    if collected.len() >= row_budget {
                        break;
                    }
                    let text = |index: usize| -> String {
                        let value: Option<&str> = r.get(index);
                        value.unwrap_or_default().to_owned()
                    };
                    let number: Option<i32> = r.get(7);
                    collected.push(
                        crate::adapters::sql_map::MapRow::new(
                            &text(0),
                            &text(1),
                            &text(2),
                            &text(3),
                            &text(4),
                            &text(5),
                            &text(6),
                        )
                        .with_number(number.map(i64::from)),
                    );
                }
            }
            // Row estimates are optional: partition statistics may be denied
            // to a least-privilege login, and that must not fail the map.
            let mut estimates: Vec<(String, i64)> = Vec::new();
            if let Ok(stream) = client.query(MAP_SIZE, &[&schema_param]).await {
                let mut rows = stream.into_row_stream();
                while let Some(r) = rows.try_next().await.unwrap_or(None) {
                    let name: Option<&str> = r.get(0);
                    let count: Option<i64> = r.get(1);
                    if let Some(name) = name {
                        estimates.push((name.to_owned(), count.unwrap_or(0)));
                    }
                }
            }
            Ok::<(Vec<crate::adapters::sql_map::MapRow>, Vec<(String, i64)>), Failure>((
                collected, estimates,
            ))
        },
        &timed,
    )) {
        Ok(value) => value,
        Err(error) => return Ok(operation_failed(error)),
    };

    let mut result = crate::adapters::sql_map::assemble("sqlserver", rows_data, limit);
    if !row_estimates.is_empty() {
        if let Some(relations) = result["relations"].as_array_mut() {
            for relation in relations.iter_mut() {
                let name = relation["name"].as_str().unwrap_or_default().to_owned();
                if let Some((_, count)) = row_estimates.iter().find(|(key, _)| *key == name) {
                    relation["rowsApprox"] = json!(count);
                }
            }
        }
    } else {
        result["sizeUnavailable"] = json!(true);
    }
    result["schemas"] = json!(if schema.is_empty() {
        "*".to_string()
    } else {
        schema.to_string()
    });
    result["readOnly"] = json!(true);
    result["visibility"] = json!("login_permissions");
    Ok(result)
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

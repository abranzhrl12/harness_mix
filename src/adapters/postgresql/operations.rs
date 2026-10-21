use super::super::project::Project;
use super::super::sql_policy::checked;
use super::catalog::catalog;
use super::config::{Config, load, required};
use super::connection::connection_plan;
use super::pool;
use harness_domain::{
    Failure,
    execution::OperationContext,
    sql::{
        SqlDescribeArgs, SqlDescribeKind, SqlMapArgs, SqlObjectType, SqlSearchArgs, SqlStatusArgs,
    },
};
use serde_json::{Value, json};
use std::time::Duration;

pub(in crate::adapters) fn status(
    project: &Project,
    args: &SqlStatusArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    let Some(config) = load(project)?.filter(|c| c.enabled) else {
        return Ok(super::super::sql_policy::not_configured("postgresql"));
    };
    let mut result = json!({"engine":"postgresql","configured":true,"connected":false,"status":"configured_unverified",
        "defaultSchema":config.default_schema,"allowedSchemas":config.allowed_schemas,"maxRows":config.max_rows,
        "maxChars":config.max_chars,"timeoutSeconds":config.timeout_seconds,"tlsMode":config.tls.mode,
        "supportedActions":["status","search","map","describe"],"querySupported":false,"schemaCacheEnabled":config.schema_cache_seconds.unwrap_or(0)>0,
        "schemaCache":{"ttlSeconds":config.schema_cache_seconds.unwrap_or(0),"maxEntries":super::super::sql_schema_cache::CAPACITY,"scope":"process","refreshSupported":true,"permissionFreshness":"cached_snapshot_when_hit"},
        "schemaCacheSecondsRequested":config.schema_cache_seconds,"readOnly":true,
        "workflow":{"overview":"map","discover":"search","inspect":"describe","parallelTool":"harness_batch","maxJobs":4,"maxConcurrent":2,"arbitraryQuerySupported":false},
        "connectionPooling":{"enabled":true,"maxConnections":pool::CAPACITY,"idleSeconds":60,"maxLifetimeSeconds":300}});
    result["setup"] = super::super::sql_policy::configuration_guide("postgresql");
    result["transportEncrypted"] = if config.tls.mode == "verify-full" {
        Value::Null
    } else {
        json!(false)
    };
    if config.tls.mode != "verify-full" {
        result["securityWarning"] = json!(
            "PostgreSQL TLS is disabled; direct remote traffic is unencrypted. Enable tls.mode=verify-full for encryption and certificate validation"
        );
    }
    result["credentialPrecedence"] = json!(["process_environment", "project_dotenv_if_configured"]);
    result["dotenvFile"] = json!(
        config
            .connection
            .as_ref()
            .and_then(|c| c.env_file.as_deref())
    );
    if let Some(connection) = config.connection.as_ref().filter(|c| {
        c.name.len() <= 128
            && c.name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    }) {
        result["connectionEnvironmentVariable"] = json!(connection.name);
        result["setup"]["requiredEnvironmentVariable"] = json!(connection.name);
    }
    if args.connect == Some(true) {
        match pool::run(catalog(
            project,
            &config,
            context,
            "SELECT current_database()::text AS database, current_user::text AS role, current_setting('server_version')::text AS version, current_setting('transaction_read_only')::text AS read_only",
            &[],
            1,
            None,
        )) {
            Ok(probe) if probe["ok"] != false => {
                result["connected"] = json!(true);
                result["transportEncrypted"] = json!(config.tls.mode == "verify-full");
                result["status"] = json!("ready");
                result["server"] = probe["results"].clone();
                result["connectionReused"] = probe["connectionReused"].clone();
            }
            Ok(probe) => {
                result["ok"] = json!(false);
                result["status"] = json!("verification_failed");
                for field in [
                    "diagnostics",
                    "timedOut",
                    "cancelled",
                    "connectionReused",
                    "connectionDiscarded",
                    "cancellationRequestSent",
                    "serverCancellationConfirmed",
                ] {
                    if let Some(value) = probe.get(field) {
                        result[field] = value.clone();
                    }
                }
            }
            Err((code, message)) => {
                super::super::sql_policy::verification_failed(&mut result, code, message);
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

const RELATIONS: &str = "SELECT n.nspname::text AS schema,c.relname::text AS name,CASE c.relkind WHEN 'v' THEN 'view' WHEN 'm' THEN 'view' ELSE 'table' END::text AS kind FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=ANY($1) AND c.relkind IN ('r','p','v','m') AND pg_catalog.has_schema_privilege(n.oid,'USAGE') AND pg_catalog.has_table_privilege(c.oid,'SELECT') AND ($3='' OR CASE WHEN c.relkind IN ('v','m') THEN 'view' ELSE 'table' END=$3) AND strpos(lower(c.relname),lower($2))>0 ORDER BY n.nspname,c.relname LIMIT $4";
const COLUMNS: &str = "SELECT n.nspname::text AS schema,c.relname::text AS name,a.attname::text AS column,'column'::text AS kind,pg_catalog.format_type(a.atttypid,a.atttypmod)::text AS data_type FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid=a.attrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=ANY($1) AND a.attnum>0 AND NOT a.attisdropped AND c.relkind IN ('r','p','v','m') AND pg_catalog.has_schema_privilege(n.oid,'USAGE') AND pg_catalog.has_column_privilege(c.oid,a.attnum,'SELECT') AND strpos(lower(a.attname),lower($2))>0 AND $3='' ORDER BY n.nspname,c.relname,a.attnum LIMIT $4";
const ROUTINES: &str = "SELECT n.nspname::text AS schema,p.proname::text AS name,CASE p.prokind WHEN 'p' THEN 'procedure' ELSE 'function' END::text AS kind,left(pg_catalog.pg_get_function_identity_arguments(p.oid),1000)::text AS identity_arguments FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname=ANY($1) AND p.prokind IN ('f','p') AND pg_catalog.has_schema_privilege(n.oid,'USAGE') AND pg_catalog.has_function_privilege(p.oid,'EXECUTE') AND strpos(lower(p.proname),lower($2))>0 AND ($3='' OR CASE p.prokind WHEN 'p' THEN 'procedure' ELSE 'function' END=$3) ORDER BY n.nspname,p.proname,p.oid LIMIT $4";

pub(in crate::adapters) fn search(
    project: &Project,
    args: &SqlSearchArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    let config = required(project)?;
    if args.query.is_empty() || args.query.len() > 128 {
        return Err(("ARGUMENT", "Catalog query must contain 1..128 bytes"));
    }
    let selected_schemas: Vec<String> = match args.schema.as_deref() {
        None | Some("*") => config.allowed_schemas.clone(),
        requested => vec![schema(&config, requested)?.to_owned()],
    };
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
        &[&selected_schemas, &args.query, &kind, &count],
        limit,
        None,
    ))?;
    result["schemas"] = json!(selected_schemas);
    result["objectTypeDefault"] =
        json!("relations; specify function/procedure/column for other catalogs");
    Ok(result)
}

const DESCRIBE: &str = "SELECT n.nspname::text AS schema,c.relname::text AS name,a.attname::text AS column,pg_catalog.format_type(a.atttypid,a.atttypmod)::text AS data_type,(NOT a.attnotnull)::text AS nullable FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid=a.attrelid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2 AND c.relkind IN ('r','p','v','m') AND a.attnum>0 AND NOT a.attisdropped AND pg_catalog.has_schema_privilege(n.oid,'USAGE') AND pg_catalog.has_column_privilege(c.oid,a.attnum,'SELECT') ORDER BY a.attnum LIMIT $3";

/// One round trip that answers the whole schema map: relations with their
/// approximate size, primary keys, foreign key edges and routine parameters.
/// Every branch is a read-only catalog projection.
const MAP: &str = r#"
SELECT kind, schema, name, detail1, detail2, detail3, detail4, num1
FROM (
  SELECT 0 AS ord, 'relation'::text AS kind, r.schema, r.name, r.kind AS detail1,
         ''::text AS detail2, ''::text AS detail3, ''::text AS detail4, r.rows_approx AS num1
  FROM (
    SELECT n.nspname::text AS schema, c.relname::text AS name,
           CASE c.relkind WHEN 'v' THEN 'view' ELSE 'table' END::text AS kind,
           GREATEST(c.reltuples, 0)::bigint::text AS rows_approx
    FROM pg_catalog.pg_class c
    JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = ANY($1) AND c.relkind IN ('r','p','v','m')
      AND pg_catalog.has_schema_privilege(n.oid,'USAGE')
      AND pg_catalog.has_table_privilege(c.oid,'SELECT')
  ) r
  UNION ALL
  SELECT 1, 'primaryKey', n.nspname::text, c.relname::text, a.attname::text, '', '', '', ''
  FROM pg_catalog.pg_constraint con
  JOIN pg_catalog.pg_class c ON c.oid = con.conrelid
  JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
  JOIN pg_catalog.pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = ANY(con.conkey)
  WHERE con.contype = 'p' AND n.nspname = ANY($1)
    AND pg_catalog.has_schema_privilege(n.oid,'USAGE')
    AND pg_catalog.has_table_privilege(c.oid,'SELECT')
  UNION ALL
  SELECT 2, 'foreignKey', n.nspname::text, con.conname::text, child.relname::text,
         child_att.attname::text, parent.relname::text, parent_att.attname::text, ''
  FROM pg_catalog.pg_constraint con
  JOIN pg_catalog.pg_class child ON child.oid = con.conrelid
  JOIN pg_catalog.pg_namespace n ON n.oid = child.relnamespace
  JOIN pg_catalog.pg_class parent ON parent.oid = con.confrelid
  JOIN LATERAL unnest(con.conkey, con.confkey) AS k(child_attnum, parent_attnum) ON TRUE
  JOIN pg_catalog.pg_attribute child_att
    ON child_att.attrelid = con.conrelid AND child_att.attnum = k.child_attnum
  JOIN pg_catalog.pg_attribute parent_att
    ON parent_att.attrelid = con.confrelid AND parent_att.attnum = k.parent_attnum
  WHERE con.contype = 'f' AND n.nspname = ANY($1)
    AND pg_catalog.has_schema_privilege(n.oid,'USAGE')
    AND pg_catalog.has_table_privilege(child.oid,'SELECT')
  UNION ALL
  SELECT 3, 'routine', n.nspname::text, p.proname::text,
         COALESCE((p.proargnames::text[])[s.i], ''),
         pg_catalog.format_type((p.proallargtypes::int[])[s.i], NULL)::text,
         COALESCE((SELECT mode::text FROM unnest(COALESCE(p.proargmodes, ARRAY[]::"char"[]))
                   WITH ORDINALITY AS u(mode, ord) WHERE ord = s.i), 'i'),
         CASE p.prokind WHEN 'p' THEN 'procedure' ELSE 'function' END::text, ''
  FROM pg_catalog.pg_proc p
  JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
  LEFT JOIN LATERAL generate_subscripts(p.proallargtypes::int[], 1) AS s(i) ON TRUE
  WHERE n.nspname = ANY($1) AND p.prokind IN ('f','p')
    AND pg_catalog.has_schema_privilege(n.oid,'USAGE')
    AND pg_catalog.has_function_privilege(p.oid,'EXECUTE')
    AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_depend d
                    WHERE d.classid = 'pg_proc'::regclass AND d.objid = p.oid AND d.deptype = 'e')
) m
ORDER BY m.ord, m.schema, m.name
LIMIT $2
"#;

pub(in crate::adapters) fn map(
    project: &Project,
    args: &SqlMapArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    let config = required(project)?;
    let selected_schemas: Vec<String> = match args.schema.as_deref() {
        None | Some("*") => config.allowed_schemas.clone(),
        requested => vec![schema(&config, requested)?.to_owned()],
    };
    let limit = args.limit.unwrap_or(50).clamp(1, 200);
    let row_budget = (limit * 8).min(1600);
    let count = row_budget as i64;
    // The map is meant to be the single answer about a schema, so it gets a
    // wider character budget than a single search or describe result.
    let mut map_config = config.clone();
    map_config.max_chars = map_config.max_chars.max(48_000);
    let result = pool::run(catalog(
        project,
        &map_config,
        context,
        MAP,
        &[&selected_schemas, &count],
        row_budget,
        None,
    ))?;
    if result["ok"] == false {
        return Ok(result);
    }
    let rows = result["results"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|row| {
            let field = |name: &str| row[name].as_str().map(str::to_owned).unwrap_or_default();
            super::super::sql_map::MapRow {
                kind: field("kind"),
                schema: field("schema"),
                name: field("name"),
                detail1: field("detail1"),
                detail2: field("detail2"),
                detail3: field("detail3"),
                detail4: field("detail4"),
                number: row["num1"].as_str().and_then(|value| value.parse().ok()),
            }
        })
        .collect::<Vec<_>>();
    let mut map = super::super::sql_map::assemble("postgresql", rows, limit);

    map["schemas"] = json!(selected_schemas);
    map["readOnly"] = json!(true);
    map["visibility"] = json!("connected_role_and_allowed_schemas");
    if result["truncated"] == true {
        map["truncated"] = json!(true);
    }
    Ok(map)
}

pub(in crate::adapters) fn describe(
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
    if name.is_empty() || name.contains('.') || selected_schema.is_empty() {
        return Err((
            "ARGUMENT",
            "Use an exact ordinary schema.object name; discover names with harness_sql search",
        ));
    }
    let count = (config.max_rows + 1) as i64;
    let ttl = config.schema_cache_seconds.unwrap_or(0);
    let timed = context.with_execution_timeout(Some(Duration::from_secs(config.timeout_seconds)));
    let mut result = pool::run(async {
        let plan = connection_plan(project, &config, &timed)?;
        let key = if ttl > 0 {
            Some(harness_domain::hash(
                json!([plan.identity, selected_schema, name, DESCRIBE])
                    .to_string()
                    .as_bytes(),
            ))
        } else {
            None
        };
        let gate = key.as_deref().map(super::super::sql_schema_cache::gate);
        let _guard = match &gate {
            Some(gate) => Some(checked(async { Ok(gate.lock().await) }, &timed).await?),
            None => None,
        };
        if let Some(key) = &key {
            if args.refresh == Some(true) {
                super::super::sql_schema_cache::invalidate(key);
            } else if let Some((mut result, age)) = super::super::sql_schema_cache::get(key) {
                timed.check()?;
                result
                    .as_object_mut()
                    .expect("catalog object")
                    .remove("connectionReused");
                result
                    .as_object_mut()
                    .expect("catalog object")
                    .remove("statementReused");
                result
                    .as_object_mut()
                    .expect("catalog object")
                    .remove("timings");
                result["protocolExchanges"] = json!(0);
                for field in [
                    "transactionMode",
                    "transactionCompletion",
                    "connectionPolicyProbe",
                ] {
                    result
                        .as_object_mut()
                        .expect("catalog object")
                        .remove(field);
                }
                result["resultCache"] = json!(true);
                result["databaseAccess"] = json!(false);
                result["schemaCache"] = json!({"enabled":true,"hit":true,"ageMs":age,"ttlSeconds":ttl,
                    "freshness":"ttl_snapshot","permissionsRevalidated":false});
                return Ok(result);
            }
        }
        let mut result = catalog(
            project,
            &config,
            &timed,
            DESCRIBE,
            &[&selected_schema, &name, &count],
            config.max_rows,
            Some(plan),
        )
        .await?;
        if result["ok"] == false {
            result["schemaCache"] = json!({"enabled":ttl>0,"hit":false,"freshness":"unavailable","permissionsRevalidated":false});
            return Ok(result);
        }
        // Never retain failures, empty visibility or incomplete/truncated schemas.
        if result["ok"] != false
            && result["truncated"] != true
            && result["count"].as_u64().is_some_and(|n| n > 0)
        {
            result["schemaHash"] = json!(harness_domain::hash(
                result["results"].to_string().as_bytes()
            ));
            if let Some(key) = key {
                super::super::sql_schema_cache::insert(
                    key,
                    result.clone(),
                    Duration::from_secs(ttl),
                );
            }
        }
        result["databaseAccess"] = json!(true);
        result["schemaCache"] = json!({"enabled":ttl>0,"hit":false,"ageMs":0,"ttlSeconds":ttl,
            "freshness":"live_statement_snapshot","permissionsRevalidated":true});
        Ok::<Value, Failure>(result)
    })?;
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

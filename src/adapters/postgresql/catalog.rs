use super::super::project::Project;
use super::super::sql_policy::{Rows, checked};
use super::config::Config;
use super::connection::{ConnectionPlan, connection_plan, pg_error, tls};
use super::pool;
use futures_util::TryStreamExt;
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tokio_postgres::types::ToSql;

pub(super) async fn catalog(
    project: &Project,
    config: &Config,
    context: &OperationContext,
    sql: &'static str,
    params: &[&(dyn ToSql + Sync)],
    limit: usize,
    plan: Option<ConnectionPlan>,
) -> Result<Value, Failure> {
    let started = Instant::now();
    let context = context.with_execution_timeout(Some(Duration::from_secs(config.timeout_seconds)));
    // The cache identity and actual connection use one immutable credential/CA snapshot.
    let ConnectionPlan {
        driver,
        certificate,
        identity,
    } = match plan {
        Some(plan) => plan,
        None => connection_plan(project, config, &context)?,
    };
    let pool_started = Instant::now();
    let mut lease = pool::checkout(identity, &context).await?;
    let pool_wait_ms = pool_started.elapsed().as_secs_f64() * 1000.0;
    let reused = lease.reused;
    let connect_started = Instant::now();
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
        let entry = lease.connection.as_ref().expect("connected lease");
        let probe = checked(async {
            entry.client.simple_query("SELECT pg_catalog.current_setting('default_transaction_read_only'), pg_catalog.current_setting('transaction_read_only'), pg_catalog.current_setting('search_path'), (SELECT setting FROM pg_catalog.pg_settings WHERE name='statement_timeout'), (SELECT setting FROM pg_catalog.pg_settings WHERE name='lock_timeout')").await.map_err(pg_error)
        }, &context).await?;
        let policy = probe
            .iter()
            .find_map(|message| match message {
                tokio_postgres::SimpleQueryMessage::Row(row) => Some(row),
                _ => None,
            })
            .ok_or((
                "PG_SESSION_POLICY",
                "Unable to verify PostgreSQL read-only session policy",
            ))?;
        let timeout = (config.timeout_seconds * 1000).to_string();
        let lock_timeout = (config.timeout_seconds * 1000).min(2000).to_string();
        if policy.get(0) != Some("on")
            || policy.get(1) != Some("on")
            || policy.get(2) != Some("pg_catalog")
            || policy.get(3) != Some(timeout.as_str())
            || policy.get(4) != Some(lock_timeout.as_str())
        {
            return Err((
                "PG_SESSION_POLICY",
                "PostgreSQL read-only, timeout or search-path session policy could not be verified",
            ));
        }
    }
    let connection_ms = if reused {
        0.0
    } else {
        connect_started.elapsed().as_secs_f64() * 1000.0
    };
    let entry = lease.connection.as_mut().expect("connected lease");
    let cancel = entry.client.cancel_token();
    let connector = entry.connector.clone();
    let result = checked(async {
        let (rows, truncated, statement_reused, prepare_ms, read_ms) = {
          let cached = entry.statements.get(sql).cloned();
          let statement_reused = cached.is_some();
          let prepare_started = Instant::now();
          let statement = match cached {
              Some(statement) => statement,
              None => {
                  let statement = entry.client.prepare(sql).await.map_err(pg_error)?;
                  if entry.statements.len() == 8 { entry.statements.clear(); }
                  entry.statements.insert(sql, statement.clone());
                  statement
              }
          };
          let prepare_ms = prepare_started.elapsed().as_secs_f64() * 1000.0;
          let read_started = Instant::now();
          let stream = entry.client.query_raw(&statement, params.iter().copied()).await.map_err(pg_error)?;
          tokio::pin!(stream);
          let mut rows = Rows::new(limit, config.max_chars);
          let mut accepting = true;
          while let Some(row) = stream.try_next().await.map_err(pg_error)? {
            context.check()?;
            // Every exposed catalog query has a bounded LIMIT (or exactly one row).
            // Drain the bounded remainder through ReadyForQuery, without retaining excess rows.
            if !accepting { continue; }
            let mut value = serde_json::Map::new();
            for (index, column) in row.columns().iter().enumerate() {
                let cell: Option<String> = row.try_get(index).map_err(|_| ("PG_RESULT", "Unexpected catalog column type"))?;
                value.insert(column.name().into(), cell.map(Value::String).unwrap_or(Value::Null));
            }
            accepting = rows.push(Value::Object(value), &context)?;
          }
          (rows.values, rows.truncated, statement_reused, prepare_ms, read_started.elapsed().as_secs_f64() * 1000.0)
        };
        Ok(json!({"engine":"postgresql","results":rows,"truncated":truncated,
            "tlsMode":config.tls.mode,"transportEncrypted":config.tls.mode=="verify-full",
            "count":rows.len(),"readOnly":true,"visibility":"connected_role_and_allowed_schemas",
            "connectionReused":reused,"statementReused":statement_reused,"resultCache":false,
            "protocolExchanges":if statement_reused {1} else {2},
            "transactionMode":"implicit_read_only","transactionCompletion":"ready_for_query",
            "connectionPolicyProbe":!reused,
            "timings":{"poolWaitMs":pool_wait_ms,"connectionSetupMs":connection_ms,
                "statementPrepareMs":prepare_ms,"queryAndReadMs":read_ms,"totalMs":started.elapsed().as_secs_f64()*1000.0},
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
            json!({"ok":false,"timedOut":super::super::sql_policy::timed_out(code),"cancelled":code=="CANCELLED",
            "diagnostics":diagnostics,"engine":"postgresql","connectionReused":reused,"connectionDiscarded":true,
            "cancellationRequestSent":sent,"serverCancellationConfirmed":false}),
        );
    }
    // RowStream yields None only after ReadyForQuery, confirming the implicit transaction ended.
    // Any failure before that point discards the session, including timeouts/cancellation.
    lease.reusable = true;
    result
}

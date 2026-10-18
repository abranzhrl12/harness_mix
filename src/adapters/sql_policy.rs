//! Shared transport-independent SQL limits, cancellation and safe result handling.
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};
use std::{future::Future, time::Duration};

pub(super) fn configuration_guide(engine: &str) -> Value {
    let pg = engine == "postgresql";
    let section = if pg { "postgresql" } else { "sqlServer" };
    let variable = if pg {
        "HARNESS_PG_CONNECTION_STRING"
    } else {
        "HARNESS_SQL_CONNECTION_STRING"
    };
    let config = if pg {
        json!({"postgresql":{"enabled":true,"allowedSchemas":["public"],"defaultSchema":"public",
            "connection":{"source":"environment","name":variable,"envFile":".env"},"tls":{"mode":"disable"},"schemaCacheSeconds":0}})
    } else {
        json!({"sqlServer":{"enabled":true,"connection":{"source":"environment"}}})
    };
    json!({"configPath":".agents/harness.config.json","configSection":section,"mergeExample":config,
        "requiredEnvironmentVariableExample":variable,"secretValuesIncluded":false,
        "steps":["Merge the example into the existing project configuration; preserve other sections",
            "Set the connection variable in the MCP server process environment or configured project dotenv file; never return its value",
            "Use a least-privilege role. PostgreSQL TLS defaults to disable; set tls.mode=verify-full for encryption and certificate validation",
            "Reconnect the MCP client if its process environment changed; then call status with connect:true"],
        "nextCall":{"tool":"harness_sql","arguments":{"action":"status","engine":engine,"connect":false}}})
}

pub(super) fn not_configured(engine: &str) -> Value {
    json!({"engine":engine,"configured":false,"connected":false,"status":"not_configured",
        "diagnostics":[{"code":"SQL_NOT_CONFIGURED","message":"Enable the selected engine in .agents/harness.config.json; follow setup, then explicitly verify the connection"}],
        "setup":configuration_guide(engine),"availableEngines":["sqlserver","postgresql"],
        "engineSelection":"Use engine=postgresql explicitly for PostgreSQL; its configuration section is postgresql, not postgres"})
}

pub(super) async fn checked<F, T>(future: F, context: &OperationContext) -> Result<T, Failure>
where
    F: Future<Output = Result<T, Failure>>,
{
    tokio::pin!(future);
    loop {
        context.check()?;
        tokio::select! {
            result = &mut future => { context.check()?; return result; },
            _ = tokio::time::sleep(Duration::from_millis(25)) => {}
        }
    }
}

/// The budget includes JSON array punctuation and escaping, measured in UTF-8 bytes.
/// A driver can still allocate a single incoming cell before we can reject its row.
pub(super) struct Rows {
    pub values: Vec<Value>,
    pub truncated: bool,
    used: usize,
    max_rows: usize,
    max_bytes: usize,
}
impl Rows {
    pub fn new(max_rows: usize, max_bytes: usize) -> Self {
        Self {
            values: Vec::new(),
            truncated: false,
            used: 2,
            max_rows,
            max_bytes,
        }
    }
    pub fn push(&mut self, mut row: Value, context: &OperationContext) -> Result<bool, Failure> {
        context.check()?;
        if self.values.len() >= self.max_rows {
            self.truncated = true;
            return Ok(false);
        }
        super::data_query::redact(&mut row, 0, &mut 0, context)?;
        let size = row.to_string().len() + usize::from(!self.values.is_empty());
        if size > self.max_bytes.saturating_sub(self.used) {
            self.truncated = true;
            return Ok(false);
        }
        self.used += size;
        self.values.push(row);
        Ok(true)
    }
}

pub(super) fn verification_failed(result: &mut Value, code: &str, message: &str) {
    result["ok"] = json!(false);
    result["connected"] = json!(false);
    result["status"] = json!("verification_failed");
    result["timedOut"] = json!(timed_out(code));
    result["cancelled"] = json!(code == "CANCELLED");
    result["diagnostics"] = json!([{"code":code,"message":message}]);
}

pub(super) fn timed_out(code: &str) -> bool {
    matches!(
        code,
        "TIMEOUT" | "SQL_LOCK_TIMEOUT" | "PG_LOCK_TIMEOUT" | "PG_STATEMENT_TIMEOUT"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_json_budget_redaction_and_row_count() {
        let context = OperationContext::without_deadline();
        let mut rows = Rows::new(2, 10);
        assert!(rows.push(json!("é"), &context).unwrap());
        assert!(rows.push(json!(1), &context).unwrap());
        assert!(!rows.push(json!(2), &context).unwrap());
        assert!(rows.truncated);
        assert!(serde_json::to_vec(&rows.values).unwrap().len() <= 10);
        let mut rows = Rows::new(10, 32);
        assert!(rows.push(json!({"password":"secret"}), &context).unwrap());
        assert_eq!(rows.values[0]["password"], "[REDACTED]");
        assert!(!rows.push(json!("x".repeat(1000)), &context).unwrap());
        assert!(serde_json::to_vec(&rows.values).unwrap().len() <= 32);
    }
    #[test]
    fn server_and_local_timeouts_have_consistent_flags() {
        for code in [
            "TIMEOUT",
            "SQL_LOCK_TIMEOUT",
            "PG_LOCK_TIMEOUT",
            "PG_STATEMENT_TIMEOUT",
        ] {
            let mut result = json!({});
            verification_failed(&mut result, code, "safe diagnostic");
            assert_eq!(result["timedOut"], true);
            assert_eq!(result["diagnostics"][0]["code"], code);
        }
        assert!(!timed_out("SQL_AUTH"));
    }
    #[tokio::test]
    async fn cancellation_and_deadline_interrupt_pending_io() {
        let expired = OperationContext::with_timeout(Duration::ZERO);
        assert_eq!(
            checked(std::future::pending::<Result<(), Failure>>(), &expired)
                .await
                .unwrap_err()
                .0,
            "TIMEOUT"
        );
        let context = OperationContext::without_deadline();
        let owner = context.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            owner.cancel();
        });
        assert_eq!(
            checked(std::future::pending::<Result<(), Failure>>(), &context)
                .await
                .unwrap_err()
                .0,
            "CANCELLED"
        );
    }
}

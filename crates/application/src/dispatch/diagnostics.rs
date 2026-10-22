use crate::{
    Application,
    ports::{SessionRepository, Workspace},
};
use serde_json::{Value, json};

pub(super) fn enrich_doctor<W: Workspace, S: SessionRepository>(
    app: &Application<W, S>,
    value: &mut Value,
) {
    value["metrics"] = app.metrics.snapshot();
    value["logging"] = app.metrics.logs.status();
    if !value["logging"]["persistenceError"].is_null() {
        if let Some(checks) = value["checks"].as_array_mut() {
            checks.push(json!({"id":"logging:persistence","status":"warning","message":"Native log archive unavailable; memory logging remains active"}));
        }
        value["counts"]["warning"] = json!(value["counts"]["warning"].as_u64().unwrap_or(0) + 1);
        if value["health"] == "ok" {
            value["health"] = json!("warning");
        }
    }
}

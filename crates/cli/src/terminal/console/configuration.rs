use harness_domain::conversation::PermissionPolicy;
use std::sync::{Arc, Mutex};

use super::{ConsoleOutput, ConsoleResult, UiState};

pub(super) fn config_text(state: &UiState, policy: PermissionPolicy) -> String {
    let agent = state
        .connection
        .as_ref()
        .map(|info| info.agent_name.as_str())
        .unwrap_or("Antigravity");
    let session = if state.session_id.is_some() {
        "activa"
    } else {
        "no iniciada"
    };
    format!(
        "Agente: {agent}\nSesión: {session}\nPermisos: {}\n",
        policy_name(policy)
    )
}

pub(super) fn policy_name(value: PermissionPolicy) -> &'static str {
    match value {
        PermissionPolicy::Ask => "ask",
        PermissionPolicy::AutoApprove => "auto",
    }
}

pub(super) async fn show_permissions(
    output: &Arc<dyn ConsoleOutput>,
    policy: &Arc<Mutex<PermissionPolicy>>,
) -> ConsoleResult<()> {
    let policy = *policy.lock().map_err(|_| "permission policy unavailable")?;
    output
        .write_text(format!(
            "Permisos: {}. Usa /permissions auto o /permissions ask.\n",
            policy_name(policy)
        ))
        .await
}

pub(super) async fn set_permissions(
    output: &Arc<dyn ConsoleOutput>,
    policy: &Arc<Mutex<PermissionPolicy>>,
    value: PermissionPolicy,
) -> ConsoleResult<()> {
    *policy.lock().map_err(|_| "permission policy unavailable")? = value;
    let message = match value {
        PermissionPolicy::Ask => "Los permisos volverán a preguntarse.\n",
        PermissionPolicy::AutoApprove => "Permisos automáticos activados para esta sesión.\n",
    };
    output.write_text(message.into()).await
}

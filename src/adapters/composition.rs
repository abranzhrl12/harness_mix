//! Constructs the native adapters and injects them into the application core.
use super::{
    NativeApplicationOptions, NativeWorkspace, disk_logs, project::Project, sessions,
    workspace::services,
};
use std::path::Path;

pub(super) fn dependencies(
    root: &Path,
    options: NativeApplicationOptions,
) -> std::io::Result<(NativeWorkspace, sessions::Sessions)> {
    let project = Project::new(root)?;
    let sessions = sessions::Sessions::new(project.clone());
    let log_store = options
        .persist_logs
        .then(|| disk_logs::DiskLogs::new(project.clone()));
    let external_scopes = std::sync::Arc::new(std::sync::Mutex::default());
    let edit_lock = std::sync::Arc::new(std::sync::Mutex::new(()));
    let workspace = NativeWorkspace {
        external_scopes: std::sync::Arc::clone(&external_scopes),
        log_store,
        source_queries: services::SourceQueryService::new(
            project.clone(),
            external_scopes,
            std::sync::Arc::clone(&edit_lock),
        ),
        source_editing: services::SourceEditingService::new(
            project.clone(),
            std::sync::Arc::clone(&edit_lock),
        ),
        session_service: services::SessionService::new(project.clone()),
        project,
        tasks: options.tasks,
        index_watch: options.index_watch,
    };

    Ok((workspace, sessions))
}

#[cfg(test)]
mod tests;

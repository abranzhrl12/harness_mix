//! Constructs the native adapters and injects them into the application core.
use super::{
    NativeApplication, NativeApplicationOptions, NativeWorkspace, disk_logs, project::Project,
    sessions,
};
use harness_application::Application;
use std::path::Path;

pub(super) fn application(
    root: &Path,
    options: NativeApplicationOptions,
) -> std::io::Result<NativeApplication> {
    let (workspace, sessions) = dependencies(root, options)?;
    Ok(Application::new(workspace, sessions))
}

fn dependencies(
    root: &Path,
    options: NativeApplicationOptions,
) -> std::io::Result<(NativeWorkspace, sessions::Sessions)> {
    let project = Project::new(root)?;
    let sessions = sessions::Sessions::new(project.clone());
    let log_store = options
        .persist_logs
        .then(|| disk_logs::DiskLogs::new(project.clone()));
    let workspace = NativeWorkspace {
        external_scopes: std::sync::Mutex::default(),
        log_store,
        project,
        edit_lock: std::sync::Mutex::new(()),
        tasks: options.tasks,
        index_watch: options.index_watch,
    };

    Ok((workspace, sessions))
}

#[cfg(test)]
mod tests;

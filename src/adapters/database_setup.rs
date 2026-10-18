//! Explicit project setup. Global npm installation never guesses which project to modify.
use super::{project::Project, sql_discovery};
use harness_domain::{Failure, sql::SqlEngine};
use serde_json::{Value, json};
use std::{io::Write, path::Path};

pub fn initialize(root: &Path) -> Result<Value, Failure> {
    let project = Project::new(root).map_err(|_| ("PATH", "Project root is unavailable"))?;
    let target = project.target_optional(".agents/harness.config.json")?;
    if target.exists() {
        return Err((
            "CONFIG_EXISTS",
            "Project configuration already exists; it was not overwritten",
        ));
    }
    let detected = sql_discovery::discover(&project, None)?;
    let config = match &detected {
        Some((SqlEngine::Postgresql, name)) => {
            json!({"postgresql":{"enabled":true,"defaultSchema":"public","allowedSchemas":["public"],
            "maxRows":50,"maxChars":8000,"timeoutSeconds":15,"schemaCacheSeconds":0,
            "connection":{"source":"environment","name":name,"envFile":".env"},"tls":{"mode":"disable"}}})
        }
        Some((SqlEngine::SqlServer, name)) => {
            json!({"sqlServer":{"enabled":true,"defaultSchema":"dbo","maxRows":50,"maxChars":8000,"timeoutSeconds":15,
            "connection":{"source":"environment","name":name,"envFile":".env"}}})
        }
        None => json!({"database":{"autoDetect":true}}),
    };
    let parent = target
        .parent()
        .ok_or(("PATH", "Invalid configuration path"))?;
    std::fs::create_dir_all(parent).map_err(|_| {
        (
            "CONFIG_WRITE",
            "Unable to create project configuration directory",
        )
    })?;
    let canonical = std::fs::canonicalize(parent)
        .map_err(|_| ("PATH", "Configuration directory is unavailable"))?;
    if !canonical.starts_with(&project.root) {
        return Err(("PATH", "Configuration directory escapes the project root"));
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .map_err(|_| {
            (
                "CONFIG_EXISTS_OR_WRITE",
                "Cannot create configuration; existing files are never overwritten",
            )
        })?;
    file.write_all(
        serde_json::to_string_pretty(&config)
            .expect("JSON template")
            .as_bytes(),
    )
    .map_err(|_| {
        (
            "CONFIG_WRITE",
            "Configuration write failed; inspect the created file",
        )
    })?;
    file.sync_all()
        .map_err(|_| ("CONFIG_WRITE", "Configuration flush failed"))?;
    Ok(
        json!({"created":true,"path":".agents/harness.config.json","credentialsWritten":false,"indexModified":false,
        "detectedEngine":detected.map(|(engine,_)| engine),"optional":true}),
    )
}

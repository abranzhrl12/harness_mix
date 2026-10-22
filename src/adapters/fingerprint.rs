//! Bounded local-workspace observations, not a Cargo dependency graph or a sandbox.
use super::project::Project;
use harness_domain::{Failure, execution::OperationContext, hash};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

const LIMIT: Failure = (
    "FINGERPRINT_LIMIT",
    "Workspace fingerprint exceeded its scan budget",
);

pub(super) fn coverage() -> serde_json::Value {
    serde_json::json!({
        "kind":"local_workspace_content_v2",
        "excludedDirectories":["target","node_modules",".git",".hg",".svn",".agents/harness/.state",".agents/harness/logs",".agents/harness/native-logs",".agents/harness/jobs",".agents/harness/proposals"],
        "externalDependenciesTracked":false,"environmentTracked":false,"current":null
    })
}

pub(super) fn capture(project: &Project, context: &OperationContext) -> Result<String, Failure> {
    let started = Instant::now();
    let mut pending = vec![PathBuf::new()];
    let mut files = Vec::new();
    let mut entries = 0;
    let mut bytes = 0usize;
    while let Some(relative) = pending.pop() {
        context.check()?;
        if started.elapsed() > Duration::from_secs(5) {
            return Err(LIMIT);
        }
        let directory =
            project.target(relative.to_str().filter(|s| !s.is_empty()).unwrap_or("."))?;
        for entry in
            fs::read_dir(directory).map_err(|_| ("FINGERPRINT_IO", "Cannot enumerate workspace"))?
        {
            context.check()?;
            if started.elapsed() > Duration::from_secs(5) {
                return Err(LIMIT);
            }
            entries += 1;
            if entries > 10000 {
                return Err(LIMIT);
            }
            let entry =
                entry.map_err(|_| ("FINGERPRINT_IO", "Cannot enumerate workspace entry"))?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or(("FINGERPRINT_PATH", "Non UTF-8 workspace path"))?;
            let path = relative.join(name);
            let text = path
                .to_str()
                .ok_or(("FINGERPRINT_PATH", "Non UTF-8 workspace path"))?;
            if text.len() > 4096 {
                return Err(LIMIT);
            }
            let normalized = text.replace('\\', "/");
            let kind = entry
                .file_type()
                .map_err(|_| ("FINGERPRINT_IO", "Cannot inspect workspace entry"))?;
            if kind.is_dir()
                && (["target", "node_modules", ".git", ".hg", ".svn"]
                    .contains(&name.to_ascii_lowercase().as_str())
                    || [
                        ".agents/harness/.state",
                        ".agents/harness/logs",
                        ".agents/harness/native-logs",
                        ".agents/harness/jobs",
                        ".agents/harness/proposals",
                    ]
                    .contains(&normalized.as_str()))
            {
                continue;
            }
            // Project validation rejects links, junctions, devices and secret paths.
            // A restricted input makes the observation unavailable, never silently complete.
            project.target(text)?;
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() {
                let content = project.bytes_with_context(text, context)?;
                bytes = bytes.checked_add(content.len()).ok_or(LIMIT)?;
                if bytes > 64 * 1024 * 1024 {
                    return Err(LIMIT);
                }
                files.push((normalized, hash(&content)));
            } else {
                return Err(("FINGERPRINT_KIND", "Unsupported workspace entry"));
            }
        }
    }
    context.check()?;
    if started.elapsed() > Duration::from_secs(5) {
        return Err(LIMIT);
    }
    files.sort_unstable();
    // Length-delimited serialization avoids collisions between path/content boundaries.
    let encoded =
        serde_json::to_vec(&files).map_err(|_| ("FINGERPRINT", "Cannot encode fingerprint"))?;
    Ok(hash(&encoded))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn content_paths_hidden_config_and_deletions_change_the_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project::new(dir.path()).unwrap();
        let capture = || capture(&project, &OperationContext::default()).unwrap();
        let empty = capture();
        fs::write(dir.path().join("a.rs"), "one").unwrap();
        let one = capture();
        assert_ne!(empty, one);
        assert_eq!(one, capture());
        fs::write(dir.path().join("a.rs"), "two").unwrap();
        let two = capture();
        assert_ne!(one, two);
        fs::rename(dir.path().join("a.rs"), dir.path().join("b.rs")).unwrap();
        assert_ne!(two, capture());
        fs::remove_file(dir.path().join("b.rs")).unwrap();
        assert_eq!(empty, capture());
        fs::create_dir(dir.path().join(".cargo")).unwrap();
        fs::write(dir.path().join(".cargo/config.toml"), "[build]").unwrap();
        assert_ne!(empty, capture());
    }
    #[test]
    fn output_is_excluded_but_secrets_and_oversized_inputs_are_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project::new(dir.path()).unwrap();
        let ctx = OperationContext::default();
        let empty = capture(&project, &ctx).unwrap();
        fs::create_dir(dir.path().join("target")).unwrap();
        fs::write(dir.path().join("target/artifact"), "ignored").unwrap();
        assert_eq!(empty, capture(&project, &ctx).unwrap());
        fs::create_dir_all(dir.path().join(".agents/harness/jobs")).unwrap();
        fs::write(
            dir.path().join(".agents/harness/jobs/jobs.db"),
            "runtime state",
        )
        .unwrap();
        assert_eq!(empty, capture(&project, &ctx).unwrap());
        fs::write(dir.path().join(".env"), "not disclosed").unwrap();
        assert!(capture(&project, &ctx).is_err());
        fs::remove_file(dir.path().join(".env")).unwrap();
        fs::write(dir.path().join("large"), vec![0u8; 2 * 1024 * 1024 + 1]).unwrap();
        assert!(capture(&project, &ctx).is_err());
    }
}

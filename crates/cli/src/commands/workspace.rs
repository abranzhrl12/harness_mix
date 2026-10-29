//! Workspace manifests configure routing, without installing clients or indexes.
use crate::WorkspaceOperation;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const MANIFEST: &str = "harness-workspace.json";

const SUPPORT_FILES: &[(&str, &str)] = &[
    (
        "harness-workspace.example.json",
        include_str!("../../templates/harness-workspace.example.json"),
    ),
    (
        "HARNESS-WORKSPACE.md",
        include_str!("../../templates/HARNESS-WORKSPACE.md"),
    ),
];

fn create_support_files(root: &Path) -> Result<Vec<Value>, String> {
    let mut report = Vec::new();
    for (name, content) in SUPPORT_FILES {
        let path = root.join(name);
        let created = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(content.as_bytes())
                    .and_then(|_| file.sync_all())
                    .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(e) => return Err(format!("cannot create {}: {e}", path.display())),
        };
        report.push(json!({"path":path,"created":created}));
    }
    Ok(report)
}

pub(super) fn discover(start: &Path) -> Result<Option<PathBuf>, String> {
    let start = start.canonicalize().map_err(|e| e.to_string())?;
    for ancestor in start.ancestors() {
        let path = ancestor.join(MANIFEST);
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                let manifest = read(&path)?;
                if manifest.projects.is_empty() {
                    return Err(format!(
                        "{} has no projects; run workspace add <name> <path>",
                        path.display()
                    ));
                }
                for entry in manifest.projects.values() {
                    directory(ancestor, Path::new(entry.path()))?;
                }
                return Ok(Some(path));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("cannot inspect workspace {}: {e}", path.display())),
        }
    }
    Ok(None)
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    projects: BTreeMap<String, Entry>,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum Entry {
    Path(String),
    Object(ProjectRoot),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProjectRoot {
    root: String,
}

impl Entry {
    fn path(&self) -> &str {
        match self {
            Self::Path(path) => path,
            Self::Object(entry) => &entry.root,
        }
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn read(path: &Path) -> Result<Manifest, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("workspace manifest must be a regular file, not a link".into());
    }
    let manifest: Manifest = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("invalid workspace manifest {}: {e}", path.display()))?;
    if manifest
        .projects
        .iter()
        .any(|(name, entry)| !valid_name(name) || entry.path().is_empty())
    {
        return Err(
            "project aliases must use ASCII letters, digits, '_' or '-'; roots must be non-empty"
                .into(),
        );
    }
    Ok(manifest)
}

fn directory(base: &Path, path: &Path) -> Result<PathBuf, String> {
    let root = base
        .join(path)
        .canonicalize()
        .map_err(|e| format!("project root {}: {e}", path.display()))?;
    if !root.is_dir() {
        return Err(format!(
            "project root {} is not a directory",
            root.display()
        ));
    }
    Ok(root)
}

// A create-new lock serializes writers; publishing by rename keeps readers from
// seeing a partial JSON document. Never overwrite an existing lock.
struct Lock {
    path: PathBuf,
    pending: Option<PathBuf>,
}
impl Drop for Lock {
    fn drop(&mut self) {
        if let Some(path) = &self.pending {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_file(&self.path);
    }
}

pub(super) fn execute(root: &Path, operation: WorkspaceOperation) -> Result<Value, String> {
    let root = directory(root, Path::new("."))?;
    let path = root.join(MANIFEST);
    let writing = matches!(
        operation,
        WorkspaceOperation::Init | WorkspaceOperation::Add { .. }
    );
    let lock_path = root.join(".harness-workspace.lock");
    let mut lock = if writing {
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .map_err(|e| format!("cannot lock workspace {}: {e}", lock_path.display()))?;
        drop(file);
        Some(Lock {
            path: lock_path,
            pending: None,
        })
    } else {
        None
    };
    let mut manifest = match fs::symlink_metadata(&path) {
        Ok(_) => read(&path)?,
        Err(e)
            if e.kind() == std::io::ErrorKind::NotFound
                && matches!(operation, WorkspaceOperation::Init) =>
        {
            Manifest::default()
        }
        Err(e) => return Err(format!("{}: {e}; run workspace init first", path.display())),
    };
    let mut changed = !path.exists();
    if let WorkspaceOperation::Add {
        name,
        path: project_path,
    } = &operation
    {
        if !valid_name(name) {
            return Err("invalid project alias; use ASCII letters, digits, '_' or '-'".into());
        }
        let canonical = directory(&root, project_path)?;
        if let Some(existing) = manifest.projects.get(name) {
            if directory(&root, Path::new(existing.path()))? != canonical {
                return Err(format!(
                    "project alias {name} already points to another root; preserved"
                ));
            }
        } else {
            for (alias, entry) in &manifest.projects {
                if directory(&root, Path::new(entry.path()))? == canonical {
                    return Err(format!("root already registered as {alias}"));
                }
            }
            let stored = project_path
                .to_str()
                .ok_or("project path must be UTF-8")?
                .replace('\\', "/");
            manifest.projects.insert(name.clone(), Entry::Path(stored));
            changed = true;
        }
    }
    if matches!(operation, WorkspaceOperation::Check) && manifest.projects.is_empty() {
        return Err("workspace has no projects; run workspace add <name> <path>".into());
    }
    let projects = manifest.projects.iter().map(|(name, entry)| {
        let resolved = directory(&root, Path::new(entry.path()));
        if matches!(operation, WorkspaceOperation::Check) {
            let resolved = resolved?;
            Ok(json!({"name":name,"root":resolved,"indexPresent":resolved.join(".codegraph/codegraph.db").is_file()}))
        } else {
            Ok(json!({"name":name,"root":entry.path(),"directoryExists":resolved.is_ok()}))
        }
    }).collect::<Result<Vec<_>, String>>()?;
    if changed {
        let bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
        let lock = lock.as_mut().ok_or("workspace mutation requires a lock")?;
        let temporary = root.join(".harness-workspace.pending");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| format!("cannot prepare workspace manifest: {e}"))?;
        lock.pending = Some(temporary.clone());
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        fs::rename(&temporary, &path).map_err(|e| format!("cannot publish workspace: {e}"))?;
        lock.pending = None;
    }
    let support_files = if matches!(operation, WorkspaceOperation::Init) {
        create_support_files(&root)?
    } else {
        Vec::new()
    };
    Ok(
        json!({"manifest":path,"changed":changed,"projects":projects,"indexesModified":false,"supportFiles":support_files}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_creates_example_and_agent_guide_without_activating_examples_or_overwriting_edits() {
        let temp = tempfile::tempdir().unwrap();
        execute(temp.path(), WorkspaceOperation::Init).unwrap();
        let example = temp.path().join("harness-workspace.example.json");
        let guide = temp.path().join("HARNESS-WORKSPACE.md");
        let template = read(&example).unwrap();
        assert_eq!(template.projects.len(), 3);
        assert_eq!(template.projects["frontend"].path(), "./frontend");
        assert!(
            read(&temp.path().join(MANIFEST))
                .unwrap()
                .projects
                .is_empty()
        );
        assert!(fs::metadata(&guide).unwrap().len() > 0);
        fs::write(&example, b"custom example").unwrap();
        fs::write(&guide, b"custom instructions").unwrap();
        execute(temp.path(), WorkspaceOperation::Init).unwrap();
        assert_eq!(fs::read(example).unwrap(), b"custom example");
        assert_eq!(fs::read(guide).unwrap(), b"custom instructions");
    }

    #[test]
    fn registration_is_idempotent_and_rejects_duplicate_root_aliases() {
        let temp = tempfile::tempdir().unwrap();
        execute(temp.path(), WorkspaceOperation::Init).unwrap();
        let add = |name: &str| WorkspaceOperation::Add {
            name: name.into(),
            path: PathBuf::from("."),
        };
        execute(temp.path(), add("backend")).unwrap();
        let path = temp.path().join(MANIFEST);
        let before = fs::read(&path).unwrap();
        assert_eq!(
            execute(temp.path(), add("backend")).unwrap()["changed"],
            false
        );
        assert!(execute(temp.path(), add("duplicate")).is_err());
        assert_eq!(fs::read(path).unwrap(), before);
    }

    #[test]
    fn writer_preserves_another_writers_lock_and_manifest() {
        let temp = tempfile::tempdir().unwrap();
        execute(temp.path(), WorkspaceOperation::Init).unwrap();
        let path = temp.path().join(MANIFEST);
        let before = fs::read(&path).unwrap();
        let lock = temp.path().join(".harness-workspace.lock");
        fs::write(&lock, b"another writer").unwrap();
        assert!(execute(temp.path(), WorkspaceOperation::Init).is_err());
        assert_eq!(fs::read(lock).unwrap(), b"another writer");
        assert_eq!(fs::read(path).unwrap(), before);
    }

    #[test]
    fn discovery_finds_nearest_manifest_from_nested_project() {
        let temp = tempfile::tempdir().unwrap();
        let nested = temp.path().join("frontend/src");
        fs::create_dir_all(&nested).unwrap();
        let manifest = temp.path().join(MANIFEST);
        fs::write(&manifest, br#"{"projects":{"frontend":"frontend"}}"#).unwrap();
        assert_eq!(
            discover(&nested).unwrap(),
            Some(manifest.canonicalize().unwrap())
        );
        let nearer = temp.path().join("frontend").join(MANIFEST);
        fs::write(&nearer, br#"{"projects":{"local":"."}}"#).unwrap();
        assert_eq!(
            discover(&nested).unwrap(),
            Some(nearer.canonicalize().unwrap())
        );
    }

    #[test]
    fn discovery_rejects_broken_manifest_instead_of_falling_back() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join(MANIFEST), b"invalid").unwrap();
        assert!(discover(temp.path()).is_err());
    }
}

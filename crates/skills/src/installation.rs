//! Project-scoped installation with a write-ahead ownership receipt.
//! Client credentials/global configuration are never copied into this receipt.
use crate::{catalog::SKILLS, lock::SyncLock, manifest, preferences, storage};
use harness_domain::hash;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

mod clients;
mod index;
const RECEIPT: &str = ".agents/harness-installation.json";
const DATABASE: &str = ".agents/harness.config.json";
const CODEGRAPH: &str = "codegraph.json";
const IGNORE: &str = ".harnessignore";

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileRecord {
    before: Option<Vec<u8>>,
    installed_hash: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    format: u32,
    root: PathBuf,
    id: String,
    files: BTreeMap<String, FileRecord>,
    clients: BTreeMap<String, clients::ClientRecord>,
    index_owned: bool,
    #[serde(default)]
    created_dirs: Vec<String>,
}

/// Holds the same skills lock through preparation and initial indexing.
pub struct Installation {
    root: PathBuf,
    receipt: Receipt,
    receipt_hash: Option<String>,
    _lock: SyncLock,
}

fn allowed(path: &str) -> bool {
    matches!(
        path,
        DATABASE | CODEGRAPH | IGNORE | manifest::PATH | preferences::PATH
    ) || SKILLS.iter().any(|skill| skill.relative_path == path)
}

fn read_receipt(root: &Path) -> Result<Option<Receipt>, String> {
    let Some(bytes) = storage::read_safe_file(root, Path::new(RECEIPT))? else {
        return Ok(None);
    };
    let receipt: Receipt =
        serde_json::from_slice(&bytes).map_err(|_| "invalid installation receipt")?;
    if receipt.format != 1 || receipt.root != root || uuid::Uuid::parse_str(&receipt.id).is_err() {
        return Err("installation receipt belongs to another root or unsupported format".into());
    }
    for (path, record) in &receipt.files {
        if !allowed(path)
            || record.installed_hash.len() != 64
            || !record.installed_hash.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("unsafe installation receipt entry".into());
        }
    }
    for key in receipt.clients.keys() {
        clients::validate_key(key)?;
    }
    for dir in &receipt.created_dirs {
        if !matches!(dir.as_str(), ".agents" | ".agents/skills" | ".codex")
            && !SKILLS
                .iter()
                .any(|s| Path::new(s.relative_path).parent() == Some(Path::new(dir)))
        {
            return Err("unsafe installation directory record".into());
        }
    }
    Ok(Some(receipt))
}

impl Installation {
    /// Open an existing installation for ownership of an explicitly requested index.
    pub fn existing(root: &Path) -> Result<Option<Self>, String> {
        let root = storage::checked_root(root)?;
        if read_receipt(&root)?.is_none() {
            return Ok(None);
        }
        let lock = SyncLock::acquire(&root)?;
        let bytes = storage::read_safe_file(&root, Path::new(RECEIPT))?
            .ok_or("installation receipt disappeared")?;
        let receipt = read_receipt(&root)?.ok_or("installation receipt disappeared")?;
        Ok(Some(Self {
            root,
            receipt,
            receipt_hash: Some(hash(&bytes)),
            _lock: lock,
        }))
    }
    pub fn begin(root: &Path, executable: &Path) -> Result<Self, String> {
        let root = storage::checked_root(root)?;
        let executable =
            fs::canonicalize(executable).map_err(|_| "current executable unavailable")?;
        if !executable.is_file() {
            return Err("current executable is not a file".into());
        }
        let agents_existed = storage::safe_metadata(&root, Path::new(".agents"))?.is_some();
        let lock = SyncLock::acquire(&root)?;
        let receipt_bytes = storage::read_safe_file(&root, Path::new(RECEIPT))?;
        let receipt = read_receipt(&root)?.unwrap_or_else(|| Receipt {
            format: 1,
            root: root.clone(),
            id: uuid::Uuid::new_v4().to_string(),
            files: BTreeMap::new(),
            clients: BTreeMap::new(),
            index_owned: false,
            created_dirs: Vec::new(),
        });
        let mut install = Self {
            root,
            receipt,
            receipt_hash: receipt_bytes.as_deref().map(hash),
            _lock: lock,
        };
        if !agents_existed && !install.receipt.created_dirs.iter().any(|d| d == ".agents") {
            install.receipt.created_dirs.push(".agents".into());
        }

        // Preflight every managed source and client before the first mutation.
        let preview = crate::sync::sync(&install.root, true, false)?;
        if preview.conflicts > 0 {
            return Err("local skill edits conflict with bundled skills; nothing installed".into());
        }
        let database = storage::read_safe_file(&install.root, Path::new(DATABASE))?;
        if let Some(bytes) = &database {
            if !serde_json::from_slice::<Value>(bytes).is_ok_and(|v| v.is_object()) {
                return Err("invalid harness.config.json; nothing installed".into());
            }
        }
        let existing_preferences = preferences::read_preferences(&install.root)?;
        let client_plans = clients::plan(&install.root, &executable, &install.receipt.clients)?;
        let mut desired = Vec::new();
        for skill in SKILLS {
            desired.push((
                skill.relative_path.to_owned(),
                skill.content.as_bytes().to_vec(),
            ));
        }
        let old_manifest = storage::read_safe_file(&install.root, Path::new(manifest::PATH))?;
        let mut files = old_manifest
            .as_deref()
            .map(serde_json::from_slice::<manifest::Manifest>)
            .transpose()
            .map_err(|_| "invalid skills manifest")?
            .map(|m| m.files)
            .unwrap_or_default();
        for skill in SKILLS {
            files.insert(
                skill.relative_path.to_owned(),
                hash(skill.content.as_bytes()),
            );
        }
        desired.push((
            manifest::PATH.into(),
            serde_json::to_vec_pretty(&manifest::Manifest {
                format_version: manifest::FORMAT,
                package_version: env!("CARGO_PKG_VERSION").into(),
                contract_version: manifest::contract_version().into(),
                files,
            })
            .map_err(|e| e.to_string())?,
        ));
        if existing_preferences.is_none() {
            desired.push((
                preferences::PATH.into(),
                serde_json::to_vec_pretty(&preferences::Preferences {
                    format_version: preferences::FORMAT,
                    auto_sync: true,
                })
                .map_err(|e| e.to_string())?,
            ));
        }
        if database.is_none() {
            desired.push((
                DATABASE.into(),
                serde_json::to_vec_pretty(&json!({"database":{"autoDetect":true}})).unwrap(),
            ));
        }
        for (path, bytes) in [
            (
                CODEGRAPH,
                b"{\n  \"include\": [],\n  \"exclude\": []\n}\n".as_slice(),
            ),
            (
                IGNORE,
                b"# Add project-specific ignore patterns here.\n".as_slice(),
            ),
        ] {
            if storage::read_safe_file(&install.root, Path::new(path))?.is_none() {
                desired.push((path.into(), bytes.to_vec()));
            }
        }
        // Record ownership/intended hashes before any file changes. A failed
        // installation can be resumed or uninstalled without guessing ownership.
        let mut expected_files = BTreeMap::new();
        for (path, bytes) in &desired {
            let current = storage::read_safe_file(&install.root, Path::new(path))?;
            if matches!(path.as_str(), CODEGRAPH | IGNORE) && current.is_some() {
                return Err(format!(
                    "configuration appeared during installation: {path}"
                ));
            }
            expected_files.insert(path.clone(), current.as_deref().map(hash));
            if current.as_deref() == Some(bytes.as_slice()) {
                continue;
            }
            if let Some(record) = install.receipt.files.get(path) {
                if current.as_deref().map(hash).as_deref() != Some(&record.installed_hash)
                    && current != record.before
                {
                    return Err(format!("modified managed file preserved: {path}"));
                }
            }
            let before = install
                .receipt
                .files
                .get(path)
                .map(|r| r.before.clone())
                .unwrap_or(current);
            install.receipt.files.insert(
                path.clone(),
                FileRecord {
                    before,
                    installed_hash: hash(bytes),
                },
            );
        }
        for plan in &client_plans {
            if let Some(record) = &plan.record {
                install
                    .receipt
                    .clients
                    .insert(plan.key.clone(), record.clone());
            }
        }
        for path in desired
            .iter()
            .map(|(path, _)| path.as_str())
            .chain(client_plans.iter().map(|p| p.key.as_str()))
        {
            let mut parent = Path::new(path).parent();
            while let Some(dir) = parent.filter(|dir| !dir.as_os_str().is_empty()) {
                if storage::safe_metadata(&install.root, dir)?.is_none() {
                    let name = dir.to_string_lossy().replace('\\', "/");
                    if !install.receipt.created_dirs.contains(&name) {
                        install.receipt.created_dirs.push(name);
                    }
                }
                parent = dir.parent();
            }
        }
        install.save()?;
        for (path, bytes) in desired {
            let current = storage::read_safe_file(&install.root, Path::new(&path))?;
            if current.as_deref() != Some(bytes.as_slice()) {
                storage::write_safe_file(
                    &install.root,
                    Path::new(&path),
                    &bytes,
                    expected_files[&path].as_deref(),
                )?;
            }
        }
        for plan in client_plans {
            plan.apply(&install.root)?;
        }
        Ok(install)
    }

    fn save(&mut self) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&self.receipt).map_err(|e| e.to_string())?;
        if bytes.len() > 1_048_576 {
            return Err(
                "installation receipt exceeds its safety budget; original files preserved".into(),
            );
        }
        storage::write_safe_file(
            &self.root,
            Path::new(RECEIPT),
            &bytes,
            self.receipt_hash.as_deref(),
        )?;
        self.receipt_hash = Some(hash(&bytes));
        Ok(())
    }

    /// Claim an absent index before asking the host to initialize it. Never adopt
    /// a pre-existing .codegraph directory, even when it happens to be empty.
    pub fn prepare_index(&mut self) -> Result<bool, String> {
        if storage::safe_metadata(&self.root, Path::new(".codegraph"))?.is_none() {
            self.receipt.index_owned = true;
            self.save()?;
            index::claim(&self.root, &self.receipt.id)?;
            return Ok(true);
        }
        if self.receipt.index_owned {
            index::verify_owner(&self.root, &self.receipt.id)?;
        }
        Ok(storage::safe_metadata(&self.root, Path::new(".codegraph/codegraph.db"))?.is_none())
    }

    pub fn report(&self) -> Value {
        json!({"mode":"init","project":self.root,"skills":"installed","clients":["codex","antigravity"],
            "configuration":"project-scoped","receipt":RECEIPT,"clientConnection":"reload_required",
            "notice":"Reload MCP servers in your client; project trust/connection approval remains under client control."})
    }

    pub fn index_owner(&self) -> Option<&str> {
        self.receipt.index_owned.then_some(self.receipt.id.as_str())
    }
}

pub fn uninstall(root: &Path) -> Result<Value, String> {
    let root = storage::checked_root(root)?;
    if read_receipt(&root)?.is_none() {
        return Ok(
            json!({"mode":"uninstall","complete":true,"removed":[],"notice":"No installation receipt; pre-existing/unowned files were preserved."}),
        );
    }
    let lock = SyncLock::acquire(&root)?;
    let mut receipt = read_receipt(&root)?.ok_or("installation receipt disappeared")?;
    if receipt.index_owned {
        index::preflight_remove(&root, &receipt.id)?;
    }
    let mut removed = Vec::new();
    let mut conflicts = Vec::new();
    for (path, record) in &receipt.files {
        let current = storage::read_safe_file(&root, Path::new(path))?;
        if current == record.before {
            continue;
        }
        let Some(bytes) = current else { continue };
        // Startup sync may have legitimately upgraded an owned skill. Accept
        // only exact bundled content, never arbitrary edits matching a manifest.
        let current_hash = hash(&bytes);
        let known_update = SKILLS
            .iter()
            .any(|s| s.relative_path == path && s.content.as_bytes() == bytes);
        if current_hash != record.installed_hash && !known_update {
            conflicts.push(path.clone());
            continue;
        }
        if let Some(before) = &record.before {
            storage::write_safe_file(&root, Path::new(path), before, Some(&current_hash))?;
        } else {
            storage::remove_safe_file(&root, Path::new(path), &current_hash)?;
        }
        removed.push(path.clone());
    }
    for (key, record) in &receipt.clients {
        match clients::remove(&root, key, record) {
            Ok(()) => removed.push(key.clone()),
            Err(_) => conflicts.push(key.clone()),
        }
    }
    if receipt.index_owned {
        index::remove(&root, &receipt.id)?;
        removed.push(".codegraph".into());
        receipt.index_owned = false;
    }
    receipt.files.retain(|path, _| conflicts.contains(path));
    receipt.clients.retain(|path, _| conflicts.contains(path));
    let old = storage::read_safe_file(&root, Path::new(RECEIPT))?
        .ok_or("installation receipt disappeared")?;
    if conflicts.is_empty() {
        storage::remove_safe_file(&root, Path::new(RECEIPT), &hash(&old))?;
    } else {
        storage::write_safe_file(
            &root,
            Path::new(RECEIPT),
            &serde_json::to_vec_pretty(&receipt).map_err(|e| e.to_string())?,
            Some(&hash(&old)),
        )?;
    }
    // Remove empty generated directories only. Never recurse through .agents,
    // .codex, skills, or any other user directory.
    drop(lock);
    receipt
        .created_dirs
        .sort_by_key(|path| std::cmp::Reverse(path.len()));
    for dir in &receipt.created_dirs {
        let _ = fs::remove_dir(root.join(dir));
    }
    Ok(
        json!({"mode":"uninstall","complete":conflicts.is_empty(),"removed":removed,"preservedModified":conflicts,
        "notice":"Shared executable, pre-existing indexes, other MCP servers and user files are not removed."}),
    )
}

#[cfg(test)]
mod tests;

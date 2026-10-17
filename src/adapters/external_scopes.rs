//! Volatile, session-bound read grants. Paths from files/results are not user consent.
use super::project::Project;
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
const MISSING: Failure = (
    "EXTERNAL_SCOPE",
    "Scope absent, expired, revoked or owned by another session",
);
type PinnedRoot = (Vec<File>, Option<File>, [u64; 2]);
#[derive(Clone)]
pub(super) struct Approved {
    canonical: PathBuf,
    project: Project,
    identity: [u64; 2],
    single: Option<String>,
}
pub(super) struct Grant {
    approved: Approved,
    id: String,
    expires: Instant,
    expires_at: i64,
}
pub(super) struct Scoped {
    pub project: Project,
    pub default_scope: Option<String>,
    _pins: Vec<File>,
    _file: Option<File>,
}
pub(super) struct Registry {
    entries: BTreeMap<(String, String), Arc<Grant>>,
    ttl: Duration,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            ttl: Duration::from_secs(3600),
        }
    }
}
impl Registry {
    fn prune(&mut self) {
        let now = Instant::now();
        self.entries.retain(|_, g| g.expires > now);
    }
    pub fn grant(
        &mut self,
        session: &str,
        approved: Approved,
        context: &OperationContext,
    ) -> Result<Value, Failure> {
        self.prune();
        let duplicate = self
            .entries
            .iter()
            .find(|((owner, _), grant)| {
                owner == session && grant.approved.canonical == approved.canonical
            })
            .map(|(key, _)| key.clone());
        let count = self
            .entries
            .keys()
            .filter(|(owner, _)| owner == session)
            .count()
            - usize::from(duplicate.is_some());
        if count >= 8 || self.entries.len() - usize::from(duplicate.is_some()) >= 128 {
            return Err((
                "EXTERNAL_SCOPE_LIMIT",
                "At most 8 scopes/session and 128 total",
            ));
        }
        context.check()?;
        if let Some(key) = duplicate {
            self.entries.remove(&key);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let grant = Arc::new(Grant {
            approved,
            id: id.clone(),
            expires: Instant::now() + self.ttl,
            expires_at: chrono::Utc::now().timestamp_millis() + self.ttl.as_millis() as i64,
        });
        let result = grant.describe();
        self.entries.insert((session.into(), id), grant);
        Ok(result)
    }
    pub fn list(&mut self, session: &str) -> Value {
        self.prune();
        json!({"scopes":self.entries.iter().filter(|((owner,_),_)|owner==session).map(|(_,g)|g.describe()).collect::<Vec<_>>()})
    }
    pub fn revoke(&mut self, session: &str, id: &str) -> Result<Value, Failure> {
        self.prune();
        self.entries
            .remove(&(session.into(), id.into()))
            .ok_or(MISSING)?;
        Ok(json!({"scopeId":id,"revoked":true}))
    }
    pub fn lookup(&mut self, session: &str, id: &str) -> Result<Arc<Grant>, Failure> {
        self.prune();
        self.entries
            .get(&(session.into(), id.into()))
            .cloned()
            .ok_or(MISSING)
    }
}
impl Grant {
    pub(super) fn describe(&self) -> Value {
        let root = self.approved.canonical.to_string_lossy();
        let root = root.strip_prefix(r"\\?\").unwrap_or(&root);
        json!({"scopeId":self.id,"root":root,"kind":if self.approved.single.is_some(){"file"}else{"directory"},"access":"read-only","expiresAt":self.expires_at,
            "notice":"Use only for the path and task explicitly authorized by the user. MCP cannot verify chat consent. Not persisted; active reads may finish after revoke."})
    }
    pub fn resolve(&self, context: &OperationContext) -> Result<Scoped, Failure> {
        context.check()?;
        if Instant::now() >= self.expires {
            return Err(MISSING);
        }
        let (pins, file, identity) = pin(&self.approved)?;
        if identity != self.approved.identity {
            return Err((
                "STALE_SCOPE",
                "Authorized target identity changed; register consent again",
            ));
        }
        context.check()?;
        Ok(Scoped {
            project: self.approved.project.clone(),
            default_scope: self.approved.single.clone(),
            _pins: pins,
            _file: file,
        })
    }
}

#[cfg(windows)]
pub(super) fn approve(input: &str, context: &OperationContext) -> Result<Approved, Failure> {
    use std::path::{Component, Path, Prefix};
    if input.len() > 4096 || input.contains('\0') || input.starts_with(r"\\") {
        return Err((
            "PATH",
            "Use a concrete absolute local drive path; no UNC/device paths",
        ));
    }
    let path = Path::new(input);
    let components: Vec<_> = path.components().collect();
    let Some(Component::Prefix(prefix)) = components.first() else {
        return Err(("PATH", "Absolute local drive path required"));
    };
    let Prefix::Disk(drive) = prefix.kind() else {
        return Err(("PATH", "Only ordinary local drive paths are accepted"));
    };
    if !path.is_absolute()
        || components.len() < 3
        || components.len() > 64
        || components.iter().any(|c| matches!(c, Component::ParentDir))
    {
        return Err((
            "PATH",
            "Authorize one concrete path, not a volume; max depth 64",
        ));
    }
    let volume_text = format!("{}:\\", drive as char);
    let wide: Vec<u16> = volume_text.encode_utf16().chain(Some(0)).collect();
    let drive_type = unsafe {
        windows::Win32::Storage::FileSystem::GetDriveTypeW(windows::core::PCWSTR(wide.as_ptr()))
    };
    if ![2, 3, 5, 6].contains(&drive_type) {
        return Err(("PATH", "Network or unavailable drives cannot be granted"));
    }
    let volume = PathBuf::from(volume_text);
    let base = Project::new(&volume).map_err(|_| ("PATH", "Drive unavailable"))?;
    let relative = path
        .strip_prefix(&volume)
        .map_err(|_| ("PATH", "Drive path mismatch"))?
        .to_str()
        .ok_or(("PATH", "UTF-8 path required"))?;
    if super::project::external_restricted(relative) {
        return Err(("PATH", "Restricted external credential/metadata root"));
    }
    let target = base.target(relative)?;
    let canonical = target
        .canonicalize()
        .map_err(|_| ("PATH", "Authorized target unavailable"))?;
    let profile = std::env::var_os("USERPROFILE")
        .and_then(|p| PathBuf::from(p).canonicalize().ok())
        .ok_or((
            "PATH",
            "Cannot validate the user-profile boundary; grant refused",
        ))?;
    if canonical == base.root || canonical == profile {
        return Err(("PATH", "Do not authorize a whole drive or user profile"));
    }
    let metadata = std::fs::symlink_metadata(&canonical)
        .map_err(|_| ("PATH", "Authorized target unavailable"))?;
    let single = if metadata.is_file() {
        let name = canonical
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or(("PATH", "UTF-8 file name required"))?
            .to_owned();
        if ["pem", "key", "pfx", "p12"]
            .iter()
            .any(|ext| name.to_ascii_lowercase().ends_with(&format!(".{ext}")))
        {
            return Err(("PATH", "Private key containers cannot be granted"));
        }
        Some(name)
    } else if metadata.is_dir() {
        None
    } else {
        return Err(("PATH", "Scope must be a regular file or directory"));
    };
    let root = if single.is_some() {
        canonical.parent().ok_or(("PATH", "Missing parent"))?
    } else {
        canonical.as_path()
    };
    let mut project = Project::new(root).map_err(|_| ("PATH", "Scope directory unavailable"))?;
    project.external = true;
    if single.is_some() {
        project.single_file = Some(canonical.clone());
    }
    let mut approved = Approved {
        canonical,
        project,
        identity: [0, 0],
        single,
    };
    let (_, _, identity) = pin(&approved)?;
    approved.identity = identity;
    context.check()?;
    Ok(approved)
}

#[cfg(windows)]
fn pin(approved: &Approved) -> Result<PinnedRoot, Failure> {
    use std::os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    };
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle},
    };
    let directory = if approved.single.is_some() {
        approved.project.root.as_path()
    } else {
        approved.canonical.as_path()
    };
    let pins = super::edit_commit::pin_directory(&approved.project, directory)
        .map_err(|_| ("PATH", "Scope ancestor is linked, replaced or inaccessible"))?;
    let file = if approved.single.is_some() {
        Some(
            File::options()
                .read(true)
                .custom_flags(0x00200000)
                .share_mode(3)
                .open(&approved.canonical)
                .map_err(|_| ("PATH", "Scope file unavailable"))?,
        )
    } else {
        None
    };
    let handle = match &file {
        Some(file) => file,
        None => pins.last().ok_or(("PATH", "Scope directory unavailable"))?,
    };
    let metadata = handle
        .metadata()
        .map_err(|_| ("PATH", "Scope metadata unavailable"))?;
    if metadata.file_attributes() & 0x400 != 0
        || (approved.single.is_some() && !metadata.is_file())
        || (approved.single.is_none() && !metadata.is_dir())
    {
        return Err(("PATH", "Scope target is not a regular file/directory"));
    }
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        GetFileInformationByHandle(HANDLE(handle.as_raw_handle()), &mut information)
            .map_err(|_| ("PATH", "Scope identity unavailable"))?;
    }
    if approved.single.is_some() && information.nNumberOfLinks != 1 {
        return Err(("PATH", "Hard-linked scope files cannot be granted"));
    }
    let identity = [
        information.dwVolumeSerialNumber as u64,
        ((information.nFileIndexHigh as u64) << 32) | information.nFileIndexLow as u64,
    ];
    Ok((pins, file, identity))
}
#[cfg(not(windows))]
pub(super) fn approve(_: &str, _: &OperationContext) -> Result<Approved, Failure> {
    Err((
        "UNSUPPORTED",
        "External scope identity/pinning is Windows-only in this release",
    ))
}
#[cfg(not(windows))]
fn pin(_: &Approved) -> Result<PinnedRoot, Failure> {
    Err(("UNSUPPORTED", "External scope identity/pinning unavailable"))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    fn approved(path: &std::path::Path) -> Approved {
        let text = path.to_string_lossy();
        approve(
            text.strip_prefix(r"\\?\").unwrap_or(&text),
            &OperationContext::default(),
        )
        .unwrap()
    }
    #[test]
    fn ttl_and_global_limits_are_bounded_without_persistent_directory_pins() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("allowed");
        std::fs::create_dir(&target).unwrap();
        let approved = approved(&target);
        let context = OperationContext::default();
        let mut expired = Registry {
            ttl: Duration::ZERO,
            ..Registry::default()
        };
        let result = expired
            .grant("session", approved.clone(), &context)
            .unwrap();
        assert!(
            expired
                .lookup("session", result["scopeId"].as_str().unwrap())
                .is_err()
        );
        let mut registry = Registry::default();
        for owner in 0..128 {
            registry
                .grant(&format!("session-{owner}"), approved.clone(), &context)
                .unwrap();
        }
        assert_eq!(
            registry
                .grant("overflow", approved, &context)
                .unwrap_err()
                .0,
            "EXTERNAL_SCOPE_LIMIT"
        );
        let mut same_owner = Registry::default();
        for index in 0..9 {
            let path = root.path().join(format!("target-{index}"));
            std::fs::create_dir(&path).unwrap();
            let result = same_owner.grant("same", self::approved(&path), &context);
            if index < 8 {
                assert!(result.is_ok());
            } else {
                assert_eq!(result.unwrap_err().0, "EXTERNAL_SCOPE_LIMIT");
            }
        }
        std::fs::rename(&target, root.path().join("renamed")).unwrap();
    }
    #[test]
    fn in_flight_resolution_pins_the_directory_until_the_read_guard_drops() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("allowed");
        std::fs::create_dir(&target).unwrap();
        let mut registry = Registry::default();
        let context = OperationContext::default();
        let result = registry
            .grant("session", approved(&target), &context)
            .unwrap();
        let grant = registry
            .lookup("session", result["scopeId"].as_str().unwrap())
            .unwrap();
        let scoped = grant.resolve(&context).unwrap();
        assert!(std::fs::rename(&target, root.path().join("renamed")).is_err());
        drop(scoped);
        std::fs::rename(&target, root.path().join("renamed")).unwrap();
    }
}

//! Safe, bounded file access rooted inside a project directory.

use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};

const MAX_MANAGED_FILE_SIZE: u64 = 1_048_576;

pub(crate) fn checked_root(root: &Path) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|error| format!("workspace root is not accessible: {error}"))?;
    if is_link_or_reparse(&metadata) || !metadata.is_dir() {
        return Err("workspace root must be a real directory, not a symlink".into());
    }
    fs::canonicalize(root).map_err(|error| format!("cannot resolve workspace root: {error}"))
}

pub(crate) fn read_safe_file(root: &Path, relative: &Path) -> Result<Option<Vec<u8>>, String> {
    match safe_metadata(root, relative)? {
        None => Ok(None),
        Some(metadata) if !metadata.is_file() => {
            Err(format!("expected a regular file at {}", relative.display()))
        }
        Some(_) => read_no_follow(&root.join(relative))
            .map(Some)
            .map_err(|error| format!("cannot read {}: {error}", relative.display())),
    }
}

pub(crate) fn write_safe_file(
    root: &Path,
    relative: &Path,
    bytes: &[u8],
    expected_hash: Option<&str>,
) -> Result<(), String> {
    let parent = create_safe_parents(root, relative)?;
    let target = root.join(relative);
    let current = read_safe_file(root, relative)?;
    match (current.as_deref(), expected_hash) {
        (None, None) => (),
        (Some(current), Some(expected)) if harness_domain::hash(current) == expected => (),
        _ => return Err(format!("concurrent file change at {}", relative.display())),
    }
    let file_name = relative
        .file_name()
        .ok_or_else(|| format!("invalid destination path: {}", relative.display()))?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        file_name.to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    let write_result = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if safe_metadata(root, relative)
            .map_err(|_| io::Error::other("unsafe destination"))?
            .is_some()
        {
            let latest = read_no_follow(&target)?;
            match expected_hash {
                Some(expected) if harness_domain::hash(&latest) == expected => (),
                _ => {
                    return Err(io::Error::other(
                        "destination changed during synchronization",
                    ));
                }
            }
        } else if expected_hash.is_some() {
            return Err(io::Error::other(
                "destination disappeared during synchronization",
            ));
        }
        fs::rename(&temporary, &target)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result.map_err(|error| format!("cannot atomically write {}: {error}", relative.display()))
}

pub(crate) fn write_new_json(
    root: &Path,
    relative: &str,
    value: &serde_json::Value,
) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("cannot encode {relative}: {error}"))?;
    write_safe_file(root, Path::new(relative), &bytes, None)
}

pub(crate) fn validate_relative(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("unsafe relative path: {}", path.display()));
    }
    Ok(())
}

pub(crate) fn safe_metadata(root: &Path, relative: &Path) -> Result<Option<fs::Metadata>, String> {
    validate_relative(relative)?;
    let mut current = root.to_path_buf();
    let components = relative.components().collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(name) = component else {
            return Err(format!("unsafe relative path: {}", relative.display()));
        };
        current.push(name);
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("cannot inspect {}: {error}", current.display())),
        };
        if is_link_or_reparse(&metadata) {
            return Err(format!(
                "refusing symlink path component: {}",
                current.display()
            ));
        }
        if index + 1 < components.len() && !metadata.is_dir() {
            return Err(format!(
                "path component is not a directory: {}",
                current.display()
            ));
        }
        if index + 1 == components.len() {
            return Ok(Some(metadata));
        }
    }
    Ok(None)
}

pub(crate) fn create_safe_parents(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    validate_relative(relative)?;
    let parent = relative
        .parent()
        .ok_or_else(|| format!("file path has no parent: {}", relative.display()))?;
    let mut current = root.to_path_buf();
    for component in parent.components() {
        let Component::Normal(name) = component else {
            return Err(format!("unsafe parent path: {}", relative.display()));
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if is_link_or_reparse(&metadata) || !metadata.is_dir() => {
                return Err(format!("unsafe directory component: {}", current.display()));
            }
            Ok(_) => (),
            Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(&current)
                .map_err(|error| format!("cannot create {}: {error}", current.display()))?,
            Err(error) => return Err(format!("cannot inspect {}: {error}", current.display())),
        }
        let canonical = fs::canonicalize(&current)
            .map_err(|error| format!("cannot resolve {}: {error}", current.display()))?;
        if !canonical.starts_with(root) {
            return Err(format!(
                "directory escapes workspace root: {}",
                current.display()
            ));
        }
    }
    Ok(root.join(parent))
}

fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn read_no_follow(path: &Path) -> io::Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if is_link_or_reparse(&metadata) || !metadata.is_file() {
        return Err(io::Error::other("refusing non-regular or reparse file"));
    }
    if metadata.len() > MAX_MANAGED_FILE_SIZE {
        return Err(io::Error::other(
            "managed file exceeds the 1 MiB safety limit",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_MANAGED_FILE_SIZE + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MANAGED_FILE_SIZE {
        return Err(io::Error::other(
            "managed file exceeds the 1 MiB safety limit",
        ));
    }
    Ok(bytes)
}

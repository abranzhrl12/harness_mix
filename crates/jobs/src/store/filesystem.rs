use super::{BUSY, DirectoryPinner, IO, MAX_FILE, PATH};
use harness_domain::Failure;
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

pub(super) fn linked(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

pub(super) fn inspect(file: &File) -> Result<(), Failure> {
    let meta = file.metadata().map_err(|_| PATH)?;
    if !meta.is_file() || linked(&meta) || meta.len() > MAX_FILE {
        return Err(PATH);
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle},
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: the borrowed file handle is live and the output structure is initialized.
        unsafe {
            GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)
                .map_err(|_| PATH)?;
        }
        if info.nNumberOfLinks != 1 {
            return Err(PATH);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            return Err(PATH);
        }
    }
    Ok(())
}

pub(super) fn file_options(exclusive: bool) -> OpenOptions {
    let mut options = File::options();
    options.read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .custom_flags(0x0020_0000)
            .share_mode(if exclusive { 0 } else { 3 });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let _ = exclusive;
        options
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .mode(0o600);
    }
    options
}

pub(super) fn checked_file(path: &Path, create: bool) -> Result<File, Failure> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if linked(&meta) || !meta.is_file() {
            return Err(PATH);
        }
    }
    let file = if create {
        match file_options(false).create_new(true).open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                file_options(false).open(path).map_err(|_| PATH)?
            }
            Err(_) => return Err(IO),
        }
    } else {
        file_options(false).open(path).map_err(|_| PATH)?
    };
    inspect(&file)?;
    Ok(file)
}

pub(super) fn sidecar_file(path: &Path) -> Result<File, Failure> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if linked(&meta) || !meta.is_file() {
            return Err(PATH);
        }
    }
    let options = || {
        let mut options = file_options(false);
        // SQLite must be able to delete a hot rollback journal during recovery.
        // The DB and ancestor pins retain deny-rename protection separately.
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(7);
        }
        options
    };
    let file = match options().create_new(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            options().open(path).map_err(|_| PATH)?
        }
        Err(_) => return Err(IO),
    };
    inspect(&file)?;
    Ok(file)
}

/// None means at least one managed directory does not exist; no directories are created.
pub(super) fn directory(
    root: &Path,
    create: bool,
    pinner: &dyn DirectoryPinner,
) -> Result<Option<(PathBuf, Vec<File>)>, Failure> {
    let root_meta = fs::symlink_metadata(root).map_err(|_| PATH)?;
    if !root_meta.is_dir() || linked(&root_meta) {
        return Err(PATH);
    }
    let canonical = root.canonicalize().map_err(|_| PATH)?;
    let mut current = canonical.clone();
    let mut pins = pinner.pin_directory(&canonical, &current)?;
    for component in [".agents", "harness", "jobs"] {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if !create {
                    return Ok(None);
                }
                match fs::create_dir(&current) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(_) => return Err(IO),
                }
            }
            Err(_) => return Err(PATH),
            Ok(_) => {}
        }
        let meta = fs::symlink_metadata(&current).map_err(|_| PATH)?;
        if !meta.is_dir() || linked(&meta) {
            return Err(PATH);
        }
        pins.extend(pinner.pin_directory(&canonical, &current)?);
    }
    Ok(Some((current, pins)))
}

pub(super) fn writer(directory: &Path) -> Result<File, Failure> {
    let path = directory.join("writer.lock");
    if let Ok(meta) = fs::symlink_metadata(&path) {
        if linked(&meta) || !meta.is_file() {
            return Err(PATH);
        }
    }
    let file = file_options(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| if lock_contended(&error) { BUSY } else { IO })?;
    inspect(&file)?;
    if file.metadata().map_err(|_| PATH)?.len() != 0 {
        return Err(PATH);
    }
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: flock borrows this live descriptor. Closing it releases the lock.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let error = std::io::Error::last_os_error();
            return Err(if lock_contended(&error) { BUSY } else { IO });
        }
    }
    Ok(file)
}

pub(super) fn lock_contended(error: &std::io::Error) -> bool {
    #[cfg(windows)]
    {
        matches!(error.raw_os_error(), Some(32 | 33)) // sharing/lock violation
    }
    #[cfg(unix)]
    {
        error.kind() == std::io::ErrorKind::WouldBlock
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = error;
        false
    }
}

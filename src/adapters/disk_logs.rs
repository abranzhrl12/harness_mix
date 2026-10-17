//! Opt-in Windows archive. Fixed managed names, pinned ancestors and one OS-exclusive writer.
#[cfg(windows)]
mod platform {
    use crate::adapters::{edit_commit::pin_directory, edit_security, project::Project};
    use harness_domain::{Failure, log_archive};
    use serde_json::Value;
    use std::os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    };
    use std::{
        fs::{self, File},
        io::{Read, Write},
        path::{Path, PathBuf},
        sync::Mutex,
    };
    use windows::{
        Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{
                BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle, MoveFileW,
            },
        },
        core::PCWSTR,
    };
    const IO: Failure = (
        "LOG_IO",
        "Native log archive I/O failed; operation outcome unchanged",
    );
    const UNSAFE: Failure = (
        "LOG_PATH",
        "Unsafe native log storage; no archive overwritten",
    );
    pub struct DiskLogs {
        directory: PathBuf,
        _pins: Vec<File>,
        _writer: File,
        saved: Mutex<u64>,
    }
    fn inspect(file: &File) -> Result<(), Failure> {
        let meta = file.metadata().map_err(|_| IO)?;
        if !meta.is_file() || meta.file_attributes() & 0x400 != 0 {
            return Err(UNSAFE);
        }
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: borrowed live handle and initialized output structure.
        unsafe {
            GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut information)
                .map_err(|_| IO)?;
        }
        if information.nNumberOfLinks != 1 {
            return Err(UNSAFE);
        }
        Ok(())
    }
    fn read(path: &Path) -> Result<Option<Value>, Failure> {
        let file = match File::options()
            .read(true)
            .custom_flags(0x00200000)
            .share_mode(7)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(UNSAFE),
        };
        inspect(&file)?;
        if file.metadata().map_err(|_| IO)?.len() > log_archive::MAX_BYTES as u64 {
            return Err(("LOG_SIZE", "Archive exceeds 256 KiB"));
        }
        let mut bytes = Vec::new();
        file.take(log_archive::MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| IO)?;
        if bytes.len() > log_archive::MAX_BYTES {
            return Err(("LOG_SIZE", "Archive exceeds 256 KiB"));
        }
        let value = serde_json::from_slice(&bytes)
            .map_err(|_| ("LOG_FORMAT", "Invalid native log JSON; no content exposed"))?;
        log_archive::validate(&value)?;
        Ok(Some(value))
    }
    fn rename(from: &Path, to: &Path) -> Result<(), Failure> {
        let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        // MoveFileW deliberately refuses an existing destination; no replacement races.
        unsafe { MoveFileW(PCWSTR(from.as_ptr()), PCWSTR(to.as_ptr())).map_err(|_| IO) }
    }
    impl DiskLogs {
        pub fn new(project: Project) -> Result<Self, Failure> {
            let mut directory = project.root.clone();
            let mut pins = pin_directory(&project, &directory).map_err(|_| UNSAFE)?;
            let mut relative = String::new();
            for part in [".agents", "harness", "native-logs"] {
                directory.push(part);
                match fs::symlink_metadata(&directory) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        fs::create_dir(&directory).map_err(|_| IO)?
                    }
                    Err(_) => return Err(UNSAFE),
                    Ok(_) => {}
                }
                if !relative.is_empty() {
                    relative.push('/');
                }
                relative.push_str(part);
                project.target(&relative).map_err(|_| UNSAFE)?;
                pins.extend(pin_directory(&project, &directory).map_err(|_| UNSAFE)?);
            }
            // An open share_mode(0) handle is released automatically on process death.
            // Keep the zero-byte lock file, so a crash cannot leave a stale create_new lock.
            let writer = File::options()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .custom_flags(0x00200000)
                .share_mode(0)
                .open(directory.join("writer.lock"))
                .map_err(|_| {
                    (
                        "LOG_BUSY",
                        "Another writer or an inaccessible native log lock",
                    )
                })?;
            inspect(&writer)?;
            if writer.metadata().map_err(|_| IO)?.len() != 0 {
                return Err(UNSAFE);
            }
            Ok(Self {
                directory,
                _pins: pins,
                _writer: writer,
                saved: Mutex::new(0),
            })
        }
        fn promote(&self) -> Result<(), Failure> {
            let current = self.directory.join("current.json");
            let previous = self.directory.join("previous.json");
            let next = self.directory.join("next.json");
            let Some(staged) = read(&next)? else {
                return Ok(());
            };
            let existing = read(&current)?;
            let backup = read(&previous)?;
            let staged_sequence = staged["lastSequence"].as_u64().ok_or(IO)?;
            if let Some(baseline) = existing.as_ref().or(backup.as_ref()) {
                let prior = baseline["lastSequence"].as_u64().ok_or(IO)?;
                if staged_sequence < prior || (existing.is_some() && staged_sequence == prior) {
                    return Err((
                        "LOG_FORMAT",
                        "Stale recovery stage; existing log history left unchanged",
                    ));
                }
            }
            if existing.is_some() {
                if backup.is_some() {
                    fs::remove_file(&previous).map_err(|_| IO)?;
                }
                rename(&current, &previous)?;
            }
            rename(&next, &current)
        }
        pub fn load(&self) -> Result<Option<Value>, Failure> {
            let mut saved = self.saved.lock().map_err(|_| IO)?;
            self.promote()?;
            let current = read(&self.directory.join("current.json"))?;
            let history = match current {
                Some(value) => Some(value),
                None => read(&self.directory.join("previous.json"))?,
            };
            *saved = history
                .as_ref()
                .and_then(|v| v["lastSequence"].as_u64())
                .unwrap_or(0);
            Ok(history)
        }
        pub fn save(&self, archive: &Value) -> Result<u64, Failure> {
            log_archive::validate(archive)?;
            let bytes = serde_json::to_vec(archive).map_err(|_| IO)?;
            if bytes.len() > log_archive::MAX_BYTES {
                return Err(("LOG_SIZE", "Archive exceeds 256 KiB"));
            }
            let mut saved = self.saved.lock().map_err(|_| IO)?;
            self.promote()?;
            let current = self.directory.join("current.json");
            let previous = self.directory.join("previous.json");
            let existing = read(&current)?;
            let backup = read(&previous)?;
            let sequence = archive["lastSequence"].as_u64().ok_or(IO)?;
            let on_disk = existing
                .as_ref()
                .and_then(|v| v["lastSequence"].as_u64())
                .unwrap_or(0);
            *saved = (*saved).max(on_disk);
            if sequence <= *saved {
                return Ok(*saved);
            }
            let next = self.directory.join("next.json");
            let mut file = File::options()
                .write(true)
                .create_new(true)
                .share_mode(0)
                .open(&next)
                .map_err(|_| IO)?;
            let written = (|| {
                inspect(&file)?;
                if existing.is_some() {
                    edit_security::protect_stage(&current, &next)?;
                } else if backup.is_some() {
                    edit_security::protect_stage(&previous, &next)?;
                }
                file.write_all(&bytes).map_err(|_| IO)?;
                file.sync_all().map_err(|_| IO)
            })();
            drop(file);
            if let Err(error) = written {
                let _ = fs::remove_file(&next);
                return Err(error);
            }
            self.promote()?;
            *saved = sequence;
            Ok(sequence)
        }
    }
}
#[cfg(windows)]
pub(super) use platform::DiskLogs;

#[cfg(not(windows))]
pub(super) struct DiskLogs;
#[cfg(not(windows))]
impl DiskLogs {
    pub fn new(_: super::project::Project) -> Result<Self, harness_domain::Failure> {
        Err((
            "UNSUPPORTED",
            "Pinned disk logging is Windows-only in this release",
        ))
    }
    pub fn load(&self) -> Result<Option<serde_json::Value>, harness_domain::Failure> {
        Err(("UNSUPPORTED", "Disk logging unavailable"))
    }
    pub fn save(&self, _: &serde_json::Value) -> Result<u64, harness_domain::Failure> {
        Err(("UNSUPPORTED", "Disk logging unavailable"))
    }
}

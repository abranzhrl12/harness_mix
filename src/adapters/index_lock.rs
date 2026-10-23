//! Windows file-sharing lock handling for the CodeGraph SQLite database.
use harness_domain::execution::OperationContext;
use std::{
    fs::File,
    io,
    os::windows::fs::OpenOptionsExt,
    path::Path,
    time::{Duration, Instant},
};

/// Pin the database while SQLite opens it, waiting only for Windows sharing
/// conflicts. Keep the returned handle alive for the caller's required scope.
pub(super) fn open_database_pin(path: &Path, context: &OperationContext) -> io::Result<File> {
    let deadline = Instant::now() + Duration::from_millis(super::INDEX_BUSY_TIMEOUT_MS);
    let mut delay = Duration::from_millis(25);
    loop {
        context
            .check()
            .map_err(|_| io::Error::new(io::ErrorKind::Interrupted, "operation cancelled"))?;
        match File::options()
            .read(true)
            .share_mode(3)
            .custom_flags(0x0020_0000)
            .open(path)
        {
            Ok(file) => return Ok(file),
            Err(error) if is_windows_lock_conflict(&error) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "index database remained locked by another process",
                    ));
                }
                std::thread::sleep(delay.min(remaining));
                delay = (delay * 2).min(Duration::from_millis(200));
            }
            Err(error) => return Err(error),
        }
    }
}

fn is_windows_lock_conflict(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(32 | 33))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_a_windows_exclusive_database_handle_until_it_is_released() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("codegraph.db");
        std::fs::write(&database, b"index").unwrap();
        let blocker = File::options()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&database)
            .unwrap();

        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(blocker);
        });
        let started = Instant::now();
        let pin = open_database_pin(&database, &OperationContext::default()).unwrap();
        release.join().unwrap();

        assert!(started.elapsed() >= Duration::from_millis(100));
        drop(pin);
    }

    #[test]
    fn does_not_retry_unrelated_open_errors() {
        let root = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let error = open_database_pin(
            &root.path().join("missing.db"),
            &OperationContext::default(),
        )
        .unwrap_err();

        assert_ne!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}

use harness_domain::Failure;
use serde_json::Value;

/// Optional durable tool log archive.
pub trait LogPersistence: Send + Sync {
    fn log_persistence_enabled(&self) -> bool {
        false
    }
    fn load_log_archive(&self) -> Result<Option<Value>, Failure> {
        Ok(None)
    }
    fn persist_log_archive(&self, _archive: &Value) -> Result<u64, Failure> {
        Err(("UNSUPPORTED", "Log persistence unavailable"))
    }
}

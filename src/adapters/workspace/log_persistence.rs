use crate::adapters::NativeWorkspace;
use harness_application::ports::workspace::LogPersistence;
use harness_domain::Failure;
use serde_json::Value;

impl LogPersistence for NativeWorkspace {
    fn log_persistence_enabled(&self) -> bool {
        self.log_store.is_some()
    }
    fn load_log_archive(&self) -> Result<Option<Value>, Failure> {
        match &self.log_store {
            Some(Ok(store)) => store.load(),
            Some(Err(error)) => Err(*error),
            None => Ok(None),
        }
    }
    fn persist_log_archive(&self, archive: &Value) -> Result<u64, Failure> {
        match &self.log_store {
            Some(Ok(store)) => store.save(archive),
            Some(Err(error)) => Err(*error),
            None => Err(("UNSUPPORTED", "Disk logging disabled")),
        }
    }
}

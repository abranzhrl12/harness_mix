//! Adaptador de infraestructura para persistencia y control de versiones / rollback.
use harness_ports::storage::StorageRepository;

pub struct LocalStorageRepository;

impl StorageRepository for LocalStorageRepository {}

use std::{env, env::VarError};

use crate::RuntimeConfig;

impl RuntimeConfig {
    /// Reads the server's runtime environment variables and applies platform policy.
    pub fn from_env(index_watch_supported: bool) -> Result<Self, &'static str> {
        Self::from_values(
            read_utf8("HARNESS_TOOL_PROFILE"),
            read_utf8("HARNESS_INDEX_WATCH"),
            env::var_os("HARNESS_MCP_EAGER_TOOLS"),
            read_utf8("HARNESS_MCP_PRESENTATION"),
            index_watch_supported,
        )
    }
}

fn read_utf8(name: &str) -> Result<Option<String>, VarError> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(VarError::NotPresent) => Ok(None),
        Err(error) => Err(error),
    }
}

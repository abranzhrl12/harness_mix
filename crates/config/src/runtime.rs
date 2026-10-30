use harness_domain::tool_contract::ToolProfile;
use std::{
    env::VarError,
    ffi::{OsStr, OsString},
};

/// Parsed runtime options consumed by the server composition layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub profile: ToolProfile,
    /// `Some` means the user explicitly set `HARNESS_INDEX_WATCH`.
    pub index_watch_setting: Option<bool>,
    pub index_watch_enabled: bool,
    pub eager_tools_enabled: bool,
    pub context_presentation: bool,
    pub structured_presentation: bool,
}

impl RuntimeConfig {
    /// Parses an environment snapshot. This is public to keep policy testable without
    /// mutating process-global environment variables.
    pub fn from_values(
        profile: Result<Option<String>, VarError>,
        index_watch: Result<Option<String>, VarError>,
        eager_tools: Option<OsString>,
        presentation: Result<Option<String>, VarError>,
        index_watch_supported: bool,
    ) -> Result<Self, &'static str> {
        let profile_name = profile
            .map_err(|_| "HARNESS_TOOL_PROFILE must be valid UTF-8")?
            .unwrap_or_else(|| "full".to_owned());
        let profile = ToolProfile::parse(&profile_name).map_err(|(_, message)| message)?;

        let index_watch_setting = match index_watch
            .map_err(|_| "HARNESS_INDEX_WATCH must be 0 or 1")?
            .as_deref()
        {
            Some("1") => Some(true),
            Some("0") => Some(false),
            None => None,
            _ => return Err("HARNESS_INDEX_WATCH must be 0 or 1"),
        };
        let index_watch_enabled =
            index_watch_setting.unwrap_or(index_watch_supported && profile == ToolProfile::Full);
        if index_watch_enabled && profile != ToolProfile::Full {
            return Err("HARNESS_INDEX_WATCH=1 requires the full tool profile");
        }
        if index_watch_enabled && !index_watch_supported {
            return Err("Automatic index publication currently requires Windows");
        }

        Ok(Self {
            profile,
            index_watch_setting,
            index_watch_enabled,
            eager_tools_enabled: eager_tools.as_deref() != Some(OsStr::new("0")),
            context_presentation: matches!(&presentation, Ok(Some(value)) if value == "context"),
            structured_presentation: !matches!(&presentation, Ok(Some(value)) if value == "legacy" || value == "context"),
        })
    }
}

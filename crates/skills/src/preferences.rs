//! Opt-in startup synchronization preferences.

use std::path::Path;

use crate::storage::{checked_root, read_safe_file, write_safe_file};

pub(crate) const PATH: &str = ".agents/harness-skills-sync.json";
pub(crate) const FORMAT: u32 = 1;

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub(crate) format_version: u32,
    pub auto_sync: bool,
}

pub(crate) fn read_preferences(root: &Path) -> Result<Option<Preferences>, String> {
    let root = checked_root(root)?;
    let bytes = read_safe_file(&root, Path::new(PATH))?;
    let Some(bytes) = bytes else { return Ok(None) };
    let preferences: Preferences = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid skills sync preferences: {error}"))?;
    if preferences.format_version != FORMAT {
        return Err(format!(
            "unsupported skills sync preferences format {}; this package supports {}",
            preferences.format_version, FORMAT
        ));
    }
    Ok(Some(preferences))
}

pub(crate) fn set_auto_sync(root: &Path, enabled: bool) -> Result<bool, String> {
    let root = checked_root(root)?;
    let relative = Path::new(PATH);
    let previous = read_preferences(&root)?;
    if previous
        .as_ref()
        .is_some_and(|preferences| preferences.auto_sync == enabled)
        || previous.is_none() && !enabled
    {
        return Ok(false);
    }
    let current = read_safe_file(&root, relative)?;
    let expected_hash = current.as_deref().map(harness_domain::hash);
    let bytes = serde_json::to_vec_pretty(&Preferences {
        format_version: FORMAT,
        auto_sync: enabled,
    })
    .map_err(|error| format!("cannot encode skills sync preferences: {error}"))?;
    write_safe_file(&root, relative, &bytes, expected_hash.as_deref())
        .map_err(|error| format!("cannot update skills sync preferences: {error}"))?;
    Ok(true)
}

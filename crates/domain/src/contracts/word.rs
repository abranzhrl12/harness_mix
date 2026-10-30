use serde::{Deserialize, Serialize};

/// Request to render and create a Word document inside the selected workspace.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct WordArgs {
    pub path: String,
    pub document: crate::word::WordDocument,
    /// Create missing parent directories inside the workspace.
    #[serde(default, rename = "createParents")]
    pub create_parents: bool,
    pub session: Option<String>,
}

impl WordArgs {
    pub fn validate(&self) -> Result<(), crate::Failure> {
        if self.path.len() > 4096
            || self.path.is_empty()
            || self.path.contains([':', '\0'])
            || self.path.starts_with(['/', '\\'])
        {
            return Err(("PATH", "Invalid workspace-relative document path"));
        }
        let normalized = self.path.replace('\\', "/");
        if normalized
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
        {
            return Err(("PATH", "Document path must stay within the workspace"));
        }
        if !normalized.to_ascii_lowercase().ends_with(".docx") {
            return Err(("ARGUMENT", "Word output path must end with .docx"));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "word_tests.rs"]
mod tests;

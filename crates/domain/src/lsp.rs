//! Read-only semantic navigation via an installed language server.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LspArgs {
    pub action: LspAction,
    pub path: String,
    /// Zero-based line, as in LSP.
    pub line: u32,
    /// Zero-based UTF-16 code units, as in LSP.
    pub character: u32,
    #[serde(default)]
    pub include_declaration: bool,
    pub session: Option<String>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum LspAction {
    Definition,
    References,
}

/// Reject out-of-range positions and positions splitting a surrogate pair.
pub fn validate_position(text: &str, line: u32, character: u32) -> Result<(), crate::Failure> {
    let line = text
        .split('\n')
        .nth(line as usize)
        .ok_or(("ARGUMENT", "LSP line is outside the document"))?
        .trim_end_matches('\r');
    let mut units = 0;
    for ch in line.chars() {
        if units == character {
            return Ok(());
        }
        units += ch.len_utf16() as u32;
        if units > character {
            return Err(("ARGUMENT", "LSP character splits a UTF-16 surrogate pair"));
        }
    }
    if units == character {
        Ok(())
    } else {
        Err(("ARGUMENT", "LSP character is outside the line"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn positions_are_utf16_and_cannot_split_a_surrogate_pair() {
        assert!(validate_position("a😀z\n", 0, 3).is_ok());
        assert_eq!(validate_position("a😀z\n", 0, 2).unwrap_err().0, "ARGUMENT");
        assert_eq!(validate_position("a😀z\n", 0, 5).unwrap_err().0, "ARGUMENT");
        assert_eq!(validate_position("a\n", 2, 0).unwrap_err().0, "ARGUMENT");
    }
}

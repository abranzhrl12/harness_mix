//! Local CodeSnap rendering for documentation and Word code-image blocks.

use codesnap::{
    config::{Background, CodeBuilder, CodeSnap, Content},
    snapshot::snapshot_data::SnapshotData,
};
use std::fmt;

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeSnapError(String);

impl CodeSnapError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for CodeSnapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for CodeSnapError {}

/// Render source code to PNG bytes with CodeSnap's built-in theme and fonts.
pub fn render_code_image(
    code: &str,
    language: Option<&str>,
) -> Result<Vec<u8>, CodeSnapError> {
    if code.trim().is_empty() {
        return Err(CodeSnapError::new("Code image source must not be empty"));
    }

    let mut code_builder = CodeBuilder::default();
    code_builder.content(code.to_owned());
    if let Some(language) = language {
        code_builder.language(language.to_owned());
    }

    let code_config = code_builder
        .build()
        .map_err(|error| CodeSnapError::new(error.to_string()))?;
    let config = CodeSnap::default()
        .content(Content::Code(code_config))
        .background(Background::Solid("#1e1e2e".into()))
        .build()
        .map_err(|error| CodeSnapError::new(error.to_string()))?;
    let snapshot = config
        .create_snapshot()
        .map_err(|error| CodeSnapError::new(error.to_string()))?;

    match snapshot
        .png_data()
        .map_err(|error| CodeSnapError::new(error.to_string()))?
    {
        SnapshotData::Image { data, .. } if data.starts_with(PNG_SIGNATURE) => Ok(data),
        SnapshotData::Image { .. } => Err(CodeSnapError::new(
            "CodeSnap returned image data without a PNG signature",
        )),
        SnapshotData::Text(_) => Err(CodeSnapError::new(
            "CodeSnap returned text instead of PNG image data",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::render_code_image;

    #[test]
    fn renders_code_to_png_bytes() {
        let png = render_code_image("fn main() {}", Some("rust"))
            .expect("a valid source snippet should render");

        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn rejects_empty_source() {
        assert!(render_code_image(" \n\t", None).is_err());
    }
}

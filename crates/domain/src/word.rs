use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct WordPreviewArgs {
    pub path: String,
    pub page: u32,
    pub session: Option<String>,
}

impl WordPreviewArgs {
    pub fn validate(&self) -> Result<(), crate::Failure> {
        let p = self.path.replace('\\', "/");
        if p.is_empty() || p.len() > 4096 || p.contains([':', '\0']) || p.starts_with('/')
            || p.split('/').any(|s| s.is_empty() || s == "." || s == "..") {
            return Err(("PATH", "Invalid workspace-relative document path"));
        }
        if !p.to_ascii_lowercase().ends_with(".docx") {
            return Err(("ARGUMENT", "Document path must end with .docx"));
        }
        if self.page == 0 || self.page > 1000 {
            return Err(("ARGUMENT", "page must be between 1 and 1000"));
        }
        Ok(())
    }
}

/// Structured content accepted by the Word document generator.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct WordDocument {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub cover: Option<CoverPage>,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub footer: Option<String>,
    #[serde(default)]
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct CoverPage {
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
}

/// Block types supported by the Word renderer.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Block {
    Paragraph {
        text: String,
        #[serde(default)]
        bold: bool,
        #[serde(default)]
        italic: bool,
    },
    Heading {
        level: u8,
        text: String,
    },
    List {
        ordered: bool,
        items: Vec<String>,
    },
    Table {
        #[serde(default)]
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Code {
        text: String,
        #[serde(default)]
        language: Option<String>,
    },
    Image {
        data_base64: String,
        mime_type: String,
        #[serde(default)]
        alt: Option<String>,
        #[serde(default)]
        width_px: Option<u32>,
    },
    #[serde(rename = "code_image")]
    CodeImage {
        code: String,
        #[serde(default)]
        language: Option<String>,
        #[serde(default)]
        alt: Option<String>,
        #[serde(default)]
        width_px: Option<u32>,
    },
    #[serde(rename = "page_break")]
    PageBreak,
}

use serde::{Deserialize, Serialize};

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

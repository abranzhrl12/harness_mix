//! Language-neutral parsing concepts and source-format classification.
mod symbol;

pub use symbol::Symbol;

use std::path::Path;

/// A source file's format, inferred from its final extension only.
///
/// This classification is intentionally pure: it does not inspect file contents
/// or select a parser implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceFormat {
    Rust,
    JavaScript,
    JavaScriptModule,
    CommonJs,
    Jsx,
    TypeScript,
    TypeScriptModule,
    TypeScriptCommonJs,
    Tsx,
    Vue,
    Astro,
    CSharp,
    Razor,
    Css,
    Scss,
    Html,
}

impl SourceFormat {
    /// Classify a path by its extension, ignoring ASCII case.
    pub fn from_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?;
        if extension.eq_ignore_ascii_case("rs") {
            Some(Self::Rust)
        } else if extension.eq_ignore_ascii_case("js") {
            Some(Self::JavaScript)
        } else if extension.eq_ignore_ascii_case("mjs") {
            Some(Self::JavaScriptModule)
        } else if extension.eq_ignore_ascii_case("cjs") {
            Some(Self::CommonJs)
        } else if extension.eq_ignore_ascii_case("jsx") {
            Some(Self::Jsx)
        } else if extension.eq_ignore_ascii_case("ts") {
            Some(Self::TypeScript)
        } else if extension.eq_ignore_ascii_case("mts") {
            Some(Self::TypeScriptModule)
        } else if extension.eq_ignore_ascii_case("cts") {
            Some(Self::TypeScriptCommonJs)
        } else if extension.eq_ignore_ascii_case("tsx") {
            Some(Self::Tsx)
        } else if extension.eq_ignore_ascii_case("vue") {
            Some(Self::Vue)
        } else if extension.eq_ignore_ascii_case("astro") {
            Some(Self::Astro)
        } else if extension.eq_ignore_ascii_case("cs") {
            Some(Self::CSharp)
        } else if extension.eq_ignore_ascii_case("cshtml") {
            Some(Self::Razor)
        } else if extension.eq_ignore_ascii_case("css") {
            Some(Self::Css)
        } else if extension.eq_ignore_ascii_case("scss") {
            Some(Self::Scss)
        } else if extension.eq_ignore_ascii_case("html") || extension.eq_ignore_ascii_case("htm") {
            Some(Self::Html)
        } else {
            None
        }
    }

    /// The language identity shared by formats with the same language family.
    pub const fn language(self) -> SourceLanguage {
        match self {
            Self::Rust => SourceLanguage::Rust,
            Self::JavaScript | Self::JavaScriptModule | Self::CommonJs | Self::Jsx => {
                SourceLanguage::JavaScript
            }
            Self::TypeScript | Self::TypeScriptModule | Self::TypeScriptCommonJs | Self::Tsx => {
                SourceLanguage::TypeScript
            }
            Self::Vue => SourceLanguage::Vue,
            Self::Astro => SourceLanguage::Astro,
            Self::CSharp => SourceLanguage::CSharp,
            Self::Razor => SourceLanguage::Razor,
            Self::Css => SourceLanguage::Css,
            Self::Scss => SourceLanguage::Scss,
            Self::Html => SourceLanguage::Html,
        }
    }
}

/// Language identifier used by parser results and protocol payloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceLanguage {
    Rust,
    JavaScript,
    TypeScript,
    Vue,
    Astro,
    CSharp,
    Razor,
    Css,
    Scss,
    Html,
}

impl SourceLanguage {
    /// Stable language spelling used in existing JSON results.
    pub const fn protocol_name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
            Self::Vue => "vue",
            Self::Astro => "astro",
            Self::CSharp => "csharp",
            Self::Razor => "razor",
            Self::Css => "css",
            Self::Scss => "scss",
            Self::Html => "html",
        }
    }
}

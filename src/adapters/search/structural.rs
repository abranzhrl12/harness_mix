use super::super::project::Failure;
use ast_grep_core::{Language, Pattern};
use ast_grep_language::SupportLang;
use std::path::Path;

/// Own the library's compiled pattern once per request, never per file.
pub(super) struct StructuralSearch {
    pub(super) language: SupportLang,
    pub(super) pattern: Pattern,
}

impl StructuralSearch {
    pub(super) fn new(language: &str, query: &str) -> Result<Self, Failure> {
        let language: SupportLang = language.parse().map_err(|_| {
            ("ARGUMENT", "unsupported ast language; e.g. javascript, typescript, tsx, rust, csharp, python, go, html, css, json")
        })?;
        let pattern = Pattern::try_new(query, language)
            .map_err(|_| ("ARGUMENT", "invalid ast pattern: provide one code expression or statement with $NAME / $$$ARGS metavariables"))?;
        // Let ast-grep interpret metavariables and grammar recovery itself:
        // e.g. JSON placeholders are intentionally not valid literal JSON.
        if pattern.has_error() {
            return Err((
                "ARGUMENT",
                "ast pattern contains syntax errors for the selected language",
            ));
        }
        Ok(Self { language, pattern })
    }

    pub(super) fn accepts(&self, path: &Path) -> bool {
        SupportLang::from_path(path) == Some(self.language)
    }
}

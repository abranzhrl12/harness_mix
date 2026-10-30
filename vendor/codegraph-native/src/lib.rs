//! Headless entry point for the pinned CodeGraph extraction walkers.
macro_rules! stack_guard {
    () => {
        if $crate::stack::exhausted() {
            return ::core::default::Default::default();
        }
    };
}
pub mod buffers;
mod csharp;
pub mod csharp_receivers;
mod docstring;
mod ids;
mod python;
#[allow(unused_mut)] // Preserve the pinned upstream source unchanged.
mod rustlang;
mod stack;
mod textutil;
mod tsjs;

mod langs {
    pub fn grammar_for(language: &str) -> Option<tree_sitter::Language> {
        match language {
            "rust" => Some(tree_sitter_rust::LANGUAGE.into()),
            "csharp" => Some(tree_sitter_c_sharp::LANGUAGE.into()),
            "typescript" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
            "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
            "javascript" | "jsx" => Some(tree_sitter_javascript::LANGUAGE.into()),
            "python" => Some(tree_sitter_python::LANGUAGE.into()),
            _ => None,
        }
    }
}

pub const LANGUAGES: &[&str] = &[
    "rust",
    "csharp",
    "typescript",
    "tsx",
    "javascript",
    "jsx",
    "python",
];

/// Strict named-import subset for the harness resolver, not an upstream walker.
/// None means the entire declaration is outside the supported runtime subset.
pub fn named_imports(signature: &str) -> Option<(String, Vec<(String, String)>)> {
    if signature.len() > 65536 {
        return None;
    }
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&langs::grammar_for("typescript")?)
        .ok()?;
    let tree = parser.parse(signature, None)?;
    let root = tree.root_node();
    if root.has_error() || root.named_child_count() != 1 {
        return None;
    }
    let declaration = root.named_child(0)?;
    if declaration.kind() != "import_statement" {
        return None;
    }
    let text = |node: tree_sitter::Node| node.utf8_text(signature.as_bytes()).ok();
    let source = text(declaration.child_by_field_name("source")?)?;
    let module = source.get(1..source.len().checked_sub(1)?)?;
    if module.contains(['\\', '\n', '\r']) {
        return None;
    }
    let mut clause = None;
    for i in 0..declaration.child_count() {
        let child = declaration.child(i)?;
        match child.kind() {
            "import" | "from" | "string" | ";" => {}
            "import_clause" => clause = Some(child),
            _ => return None, // type-only, attributes and comments are deferred.
        }
    }
    let clause = clause?;
    if clause.named_child_count() != 1 {
        return None;
    }
    let named = clause.named_child(0)?;
    if named.kind() != "named_imports" {
        return None;
    }
    let mut bindings = Vec::new();
    for i in 0..named.named_child_count() {
        let spec = named.named_child(i)?;
        if spec.kind() != "import_specifier" {
            return None;
        }
        for j in 0..spec.child_count() {
            if !matches!(spec.child(j)?.kind(), "identifier" | "as") {
                return None;
            }
        }
        let original = spec.child_by_field_name("name")?;
        let local = spec.child_by_field_name("alias").unwrap_or(original);
        bindings.push((text(original)?.to_owned(), text(local)?.to_owned()));
        if bindings.len() > 256 {
            return None;
        }
    }
    Some((module.to_owned(), bindings))
}

/// Deliberately stronger than lexical scope analysis: outside imports, every
/// occurrence must be a bare call. Any declaration/parameter/pattern or other
/// use defers the whole binding. This prevents imported aliases capturing locals.
pub fn unshadowed_import_calls(source: &str, language: &str, name: &str) -> bool {
    if source.len() > 1024 * 1024 {
        return false;
    }
    let Some(grammar) = langs::grammar_for(language) else {
        return false;
    };
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&grammar).is_err() {
        return false;
    }
    let Some(tree) = parser.parse(source, None) else {
        return false;
    };
    if tree.root_node().has_error() {
        return false;
    }
    let mut cursor = tree.walk();
    let mut visited = 0;
    loop {
        visited += 1;
        if visited > 100_000 {
            return false;
        }
        let node = cursor.node();
        if node.kind() != "import_statement" {
            if node.child_count() == 0 && node.is_named() {
                let text = node.utf8_text(source.as_bytes()).unwrap_or("");
                if text == "eval" {
                    return false;
                }
                if text == name {
                    let Some(parent) = node.parent() else {
                        return false;
                    };
                    if node.kind() != "identifier"
                        || parent.kind() != "call_expression"
                        || parent.child_by_field_name("function") != Some(node)
                    {
                        return false;
                    }
                }
            }
            if cursor.goto_first_child() {
                continue;
            }
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return true;
            }
        }
    }
}

/// First resolver slice accepts explicit top-level named function exports only.
/// `export default function f` does NOT export a named `f` binding.
pub fn named_function_export(source: &str, language: &str, name: &str) -> bool {
    if source.len() > 1024 * 1024 {
        return false;
    }
    let Some(grammar) = langs::grammar_for(language) else {
        return false;
    };
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&grammar).is_err() {
        return false;
    }
    let Some(tree) = parser.parse(source, None) else {
        return false;
    };
    let root = tree.root_node();
    if root.has_error() {
        return false;
    }
    for i in 0..root.named_child_count() {
        let Some(export) = root.named_child(i) else {
            return false;
        };
        if export.kind() != "export_statement" {
            continue;
        }
        if (0..export.child_count()).any(|j| export.child(j).is_some_and(|n| n.kind() == "default"))
        {
            continue;
        }
        let Some(declaration) = export.child_by_field_name("declaration") else {
            continue;
        };
        if declaration.kind() != "function_declaration" {
            continue;
        }
        if declaration
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            == Some(name)
        {
            return true;
        }
    }
    false
}

pub fn extract(path: &str, source: &str, language: &str) -> Result<buffers::EmitOut, String> {
    if source.len() > 1024 * 1024 {
        return Err("source size budget exceeded".into());
    }
    stack::run_guarded(|| match language {
        "rust" => rustlang::extract(path, source),
        "csharp" => csharp::extract(path, source),
        "typescript" | "tsx" | "javascript" | "jsx" => tsjs::extract(path, source, language),
        "python" => python::extract(path, source),
        _ => Err("unsupported native extraction language".into()),
    })
}

#[cfg(test)]
mod iife_regression_tests {
    #[test]
    fn anonymous_iife_is_not_emitted_as_a_named_call() {
        let graph = super::extract("sample.js", "(function () { useful(); })();", "javascript").unwrap();
        let arena = String::from_utf8_lossy(&graph.arena);
        assert!(arena.contains("useful"));
        assert!(!arena.contains("(function () { useful(); })"));
    }
}

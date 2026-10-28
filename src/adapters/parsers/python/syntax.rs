//! Python AST syntax validation using tree-sitter via ast-grep.
//! Guarantees syntax safety before edits are committed in harness_replace / harness_write.

use ast_grep_language::SupportLang;
use harness_domain::{Failure, execution::OperationContext};

pub(in crate::adapters) const DEFAULT_SOURCE_LIMIT_BYTES: usize = 512 * 1024;

pub(in crate::adapters) fn validate_syntax(
    source: &str,
    context: &OperationContext,
) -> Result<(), Failure> {
    context.check()?;
    if source.len() > DEFAULT_SOURCE_LIMIT_BYTES {
        return Err(("PARSER_LIMIT", "Python parser limited to 512 KiB"));
    }
    let tree = ast_grep_core::AstGrep::try_new(source, SupportLang::Python)
        .map_err(|_| ("SYNTAX_ERROR", "Python source is not syntactically valid"))?;
    context.check()?;
    if tree.root().get_doc().tree.root_node().has_error() {
        return Err(("SYNTAX_ERROR", "Python source is not syntactically valid"));
    }
    Ok(())
}

/// Reparse only after a rejected candidate to report the first parser span.
pub(in crate::adapters) fn first_error_location(source: &str) -> Option<(usize, usize)> {
    let tree = ast_grep_core::AstGrep::try_new(source, SupportLang::Python).ok()?;
    let root = tree.root().get_doc().tree.root_node();
    if !root.has_error() {
        return None;
    }
    let mut cursor = root.walk();
    loop {
        let node = cursor.node();
        if node.is_error() || node.is_missing() {
            let pos = node.start_position();
            return Some((pos.row + 1, pos.column + 1));
        }
        if node.has_error() && cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                let pos = root.start_position();
                return Some((pos.row + 1, pos.column + 1));
            }
        }
    }
}

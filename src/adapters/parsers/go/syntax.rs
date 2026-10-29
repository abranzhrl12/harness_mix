use ast_grep_language::SupportLang;
use harness_domain::{Failure, execution::OperationContext};

pub(in crate::adapters) const DEFAULT_SOURCE_LIMIT_BYTES: usize = 512 * 1024;

pub(in crate::adapters) fn validate_syntax(
    source: &str,
    context: &OperationContext,
) -> Result<(), Failure> {
    context.check()?;
    if source.len() > DEFAULT_SOURCE_LIMIT_BYTES {
        return Err(("PARSER_LIMIT", "Go parser limited to 512 KiB"));
    }
    let tree = ast_grep_core::AstGrep::try_new(source, SupportLang::Go)
        .map_err(|_| ("SYNTAX_ERROR", "Go source is not syntactically valid"))?;
    context.check()?;
    if tree.root().get_doc().tree.root_node().has_error() {
        return Err(("SYNTAX_ERROR", "Go source is not syntactically valid"));
    }
    let root = tree.root().get_doc().tree.root_node();
    let first_declaration = (0..root.named_child_count())
        .filter_map(|index| root.named_child(index))
        .find(|node| node.kind() != "comment");
    if first_declaration.is_none_or(|node| node.kind() != "package_clause") {
        return Err(("SYNTAX_ERROR", "Go source must begin with a package clause"));
    }
    Ok(())
}

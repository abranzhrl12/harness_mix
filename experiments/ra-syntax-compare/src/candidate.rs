//! Experimental navigation adapter. Not a production backend or graph resolver.
use harness_domain::{hash, parsing::Symbol, results::prefix, Failure};
use ra_ap_syntax::{ast, AstNode, AstToken, Edition, SourceFile, SyntaxKind, SyntaxNode};

pub fn collect(source: &str) -> Result<Vec<Symbol>, Failure> {
    if source.len() > 256 * 1024 {
        return Err(("PARSER_LIMIT", "Rust parser limited to 256 KiB"));
    }
    let mut depth = 0usize;
    for byte in source.bytes() {
        if matches!(byte, b'(' | b'{' | b'[') { depth += 1; }
        else if matches!(byte, b')' | b'}' | b']') { depth = depth.saturating_sub(1); }
        if depth > 64 { return Err(("PARSER_LIMIT", "Conservative Rust nesting budget exceeded")); }
    }
    let base = usize::from(source.starts_with('\u{feff}')) * 3;
    let input = &source[base..];
    let prepared = if input.starts_with("#!") && !input.starts_with("#![") {
        let end = input.find('\n').unwrap_or(input.len());
        format!("{}{}", " ".repeat(end), &input[end..])
    } else {
        input.to_owned()
    };
    let parsed = SourceFile::parse(&prepared, Edition::Edition2024);
    if !parsed.errors().is_empty() {
        return Err(("SYNTAX_ERROR", "Rust source is not syntactically valid"));
    }
    let mut output = Vec::new();
    let digest = hash(source.as_bytes());
    walk(parsed.tree().syntax(), "", source, base, &digest, &mut output)?;
    Ok(output)
}

fn walk(parent: &SyntaxNode, container: &str, source: &str, base: usize,
        digest: &str, output: &mut Vec<Symbol>) -> Result<(), Failure> {
    use SyntaxKind::*;
    for node in parent.children() {
        let kind = match node.kind() {
            FN => if parent.kind() == ASSOC_ITEM_LIST
                && parent.parent().is_some_and(|n| n.kind() == TRAIT)
                && !node.children().any(|n| n.kind() == BLOCK_EXPR) {
                    "function_signature_item"
                } else { "function_item" },
            STRUCT => "struct_item", ENUM => "enum_item", UNION => "union_item",
            TRAIT | TRAIT_ALIAS => "trait_item", MODULE => "mod_item",
            TYPE_ALIAS => "type_item", CONST => "const_item", STATIC => "static_item",
            MACRO_RULES => "macro_definition", IMPL => "impl_item", _ => continue,
        };
        if output.len() >= 1000 { return Err(("PARSER_LIMIT", "Symbol budget exceeded")); }
        let name = if let Some(item) = ast::Impl::cast(node.clone()) {
            let ty = item.self_ty().ok_or(("RANGE", "Missing self type"))?.syntax().text().to_string();
            if let Some(trait_) = item.trait_() {
                let negative = node.children_with_tokens().any(|e| e.as_token().is_some_and(|t| t.text() == "!"));
                format!("impl {}{} for {ty}", if negative { "!" } else { "" }, trait_.syntax().text())
            } else { format!("impl {ty}") }
        } else {
            node.children().find_map(ast::Name::cast)
                .ok_or(("RANGE", "Missing declaration name"))?.text().to_string()
        };
        let range = node.text_range();
        // Rowan attaches ordinary leading comments to declarations; syn does not.
        let start = node.children_with_tokens().find(|element| match element {
            ra_ap_syntax::NodeOrToken::Node(_) => true,
            ra_ap_syntax::NodeOrToken::Token(t) => t.kind() != WHITESPACE
                && !ast::Comment::cast(t.clone()).is_some_and(|c| !c.is_doc()),
        }).map(|e| usize::from(e.text_range().start())).unwrap_or(usize::from(range.start()));
        let bytes = start + base..usize::from(range.end()) + base;
        let mut signature_start = bytes.start;
        let mut docs = Vec::new();
        for element in node.children_with_tokens() {
            match element {
                ra_ap_syntax::NodeOrToken::Node(n) if n.kind() == ATTR => {
                    signature_start = usize::from(n.text_range().end()) + base;
                    if n.text().to_string().trim_start().starts_with("#[doc") {
                        if let Some(value) = n.descendants_with_tokens()
                            .filter_map(|e| e.into_token()).find_map(ast::String::cast)
                            .and_then(|s| s.value().ok().map(|v| v.into_owned())) {
                            docs.push(value);
                        }
                    }
                }
                ra_ap_syntax::NodeOrToken::Token(t) => {
                    if let Some(comment) = ast::Comment::cast(t) {
                        if let Some(doc) = comment.doc_comment() {
                            signature_start = usize::from(comment.syntax().text_range().end()) + base;
                            docs.push(doc.replace("\r\n", "\n").trim_end_matches('\r').to_owned());
                        }
                    }
                }
                _ => {}
            }
        }
        let body = node.children().find(|n| matches!(n.kind(),
            BLOCK_EXPR | RECORD_FIELD_LIST | VARIANT_LIST | ASSOC_ITEM_LIST | ITEM_LIST | TOKEN_TREE));
        let end = body.as_ref().map(|n| usize::from(n.text_range().start()) + base).unwrap_or(bytes.end);
        let compact = source.get(signature_start..end).ok_or(("RANGE", "Invalid signature"))?
            .split_whitespace().collect::<Vec<_>>().join(" ");
        let signature = if body.is_some() { format!("{} …", prefix(&compact, 398)) }
            else { prefix(&compact, 400).to_owned() };
        let from = source[..bytes.start].encode_utf16().count();
        let to = from + source[bytes.clone()].encode_utf16().count();
        output.push(Symbol {
            name: name.clone(), container: container.to_owned(), kind,
            start: source[..bytes.start].bytes().filter(|b| *b == b'\n').count() + 1,
            end: source[..bytes.end].bytes().filter(|b| *b == b'\n').count()
                + usize::from(!source[..bytes.end].ends_with('\n')),
            signature, doc: prefix(&docs.join("\n"), 600).to_owned(),
            id: hash(format!("{digest}:{from}:{to}").as_bytes())[..24].to_owned(), bytes,
        });
        if matches!(node.kind(), MODULE | IMPL | TRAIT) {
            let child = if container.is_empty() { name } else { format!("{container}::{name}") };
            if let Some(list) = node.children().find(|n| matches!(n.kind(), ITEM_LIST | ASSOC_ITEM_LIST)) {
                walk(&list, &child, source, base, digest, output)?;
            }
        }
    }
    Ok(())
}

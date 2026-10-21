/// Canonical declaration kinds shared by the live parsers and the graph.
///
/// The graph and the language-specific parsers name the same constructs
/// differently (`function` vs `function_item` vs `function_declaration`), so
/// both sides are normalized to one canonical vocabulary before matching.
pub(in crate::adapters) fn canonical_kind(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "function_item"
        | "function"
        | "function_declaration"
        | "function_definition"
        | "method"
        | "method_definition"
        | "method_signature"
        | "constructor"
        | "razor_helper" => "function_item",
        "function_signature_item" => "function_signature_item",
        "struct_item" | "struct" | "class" | "class_declaration" | "record" => "struct_item",
        "enum_item" | "enum" | "enum_declaration" => "enum_item",
        "union_item" | "union" => "union_item",
        "trait_item" | "trait" | "interface" | "interface_declaration" => "trait_item",
        "mod_item" | "module" | "namespace" | "internal_module" | "razor_section" => "mod_item",
        "type_item" | "type" | "type_alias_declaration" | "razor_model" => "type_item",
        "const_item" | "const" | "constant" => "const_item",
        "static_item" | "static" => "static_item",
        "macro_definition" | "macro" => "macro_definition",
        "property_item" | "property" | "property_declaration" => "property_item",
        "variable_item" | "variable" | "field" | "variable_declarator" => "variable_item",
        "impl_item" => "impl_item",
        _ => return None,
    })
}

/// Every kind a selector may ask for: canonical names and their aliases.
pub(in crate::adapters) const SELECTOR_KINDS: &[&str] = &[
    "razor_helper",
    "razor_section",
    "razor_model",
    "function_item",
    "function_signature_item",
    "struct_item",
    "enum_item",
    "union_item",
    "trait_item",
    "mod_item",
    "type_item",
    "const_item",
    "static_item",
    "macro_definition",
    "property_item",
    "variable_item",
    "impl_item",
    "function",
    "function_declaration",
    "function_definition",
    "method",
    "method_definition",
    "method_signature",
    "constructor",
    "struct",
    "class",
    "class_declaration",
    "record",
    "enum",
    "enum_declaration",
    "union",
    "trait",
    "interface",
    "interface_declaration",
    "module",
    "namespace",
    "internal_module",
    "type",
    "type_alias_declaration",
    "const",
    "constant",
    "static",
    "macro",
    "property",
    "property_declaration",
    "variable",
    "field",
    "variable_declarator",
];

/// Concrete graph node kinds a canonical kind maps to.
///
/// `impl_item` has no graph representation: Rust impl blocks are not emitted as
/// nodes, so the graph cannot answer that selector and the live path is used.
pub(in crate::adapters) fn graph_kinds(canonical: &str) -> &'static [&'static str] {
    match canonical {
        "function_item" => &["function", "method", "constructor"],
        "function_signature_item" => &["function", "method"],
        "struct_item" => &["struct", "class", "record"],
        "enum_item" => &["enum"],
        "union_item" => &["union"],
        "trait_item" => &["trait", "interface"],
        "mod_item" => &["module", "namespace"],
        "type_item" => &["type"],
        "const_item" => &["constant", "const"],
        "static_item" => &["static"],
        "macro_definition" => &["macro"],
        "property_item" => &["property"],
        "variable_item" => &["variable", "field"],
        _ => &[],
    }
}

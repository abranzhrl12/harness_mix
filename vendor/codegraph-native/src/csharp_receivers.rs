//! Conservative lexical receiver bindings, separate from the pinned extractor ABI.
use std::collections::HashMap;
use tree_sitter::{Node, Parser};

#[derive(Clone, Debug)]
pub struct Call {
    pub name: String,
    pub type_name: String,
    pub method: String,
    pub line: usize,
    pub column: usize,
}
struct Binding {
    ty: Option<String>,
    scope: std::ops::Range<usize>,
    declaration: usize,
    field: bool,
}
struct BindingIndex {
    bindings: Vec<Binding>,
    max_end: Vec<usize>,
    #[cfg(test)]
    visited: std::cell::Cell<usize>,
}
impl BindingIndex {
    fn new(mut bindings: Vec<Binding>) -> Self {
        bindings.sort_unstable_by_key(|binding| binding.scope.start);
        let mut index = Self {
            max_end: vec![0; bindings.len()],
            bindings,
            #[cfg(test)]
            visited: std::cell::Cell::new(0),
        };
        index.build(0, index.bindings.len());
        index
    }
    fn build(&mut self, start: usize, end: usize) -> usize {
        if start == end {
            return 0;
        }
        let middle = start + (end - start) / 2;
        let left = self.build(start, middle);
        let right = self.build(middle + 1, end);
        let maximum = self.bindings[middle].scope.end.max(left).max(right);
        self.max_end[middle] = maximum;
        maximum
    }
    fn select(
        &self,
        point: usize,
        explicit_this: bool,
        class_scope: Option<(usize, usize)>,
    ) -> Option<&Binding> {
        #[cfg(test)]
        self.visited.set(0);
        let mut selection = (None, false);
        self.query(
            0,
            self.bindings.len(),
            point,
            explicit_this,
            class_scope,
            &mut selection,
        );
        match selection {
            (Some(binding), false) => Some(binding),
            _ => None,
        }
    }
    fn query<'a>(
        &'a self,
        start: usize,
        end: usize,
        point: usize,
        explicit_this: bool,
        class_scope: Option<(usize, usize)>,
        selection: &mut (Option<&'a Binding>, bool),
    ) {
        if start == end {
            return;
        }
        #[cfg(test)]
        self.visited.set(self.visited.get() + 1);
        let middle = start + (end - start) / 2;
        if self.max_end[middle] <= point {
            return;
        }
        self.query(start, middle, point, explicit_this, class_scope, selection);
        let binding = &self.bindings[middle];
        if binding.scope.start > point {
            return;
        }
        if binding.scope.end > point
            && (!explicit_this || binding.field)
            && (!binding.field || class_scope == Some((binding.scope.start, binding.scope.end)))
        {
            let key = (binding.scope.len(), binding.field);
            match selection.0 {
                None => *selection = (Some(binding), false),
                Some(best) if key < (best.scope.len(), best.field) => {
                    *selection = (Some(binding), false)
                }
                Some(best) if binding.scope == best.scope && binding.field == best.field => {
                    selection.1 = true
                }
                _ => {}
            }
        }
        self.query(
            middle + 1,
            end,
            point,
            explicit_this,
            class_scope,
            selection,
        );
    }
}
fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}
fn simple_type(value: &str) -> Option<String> {
    if value.len() > 512 {
        return None;
    }
    let bare = value.strip_prefix("global::").unwrap_or(value);
    (!matches!(bare, "var" | "dynamic" | "object")
        && bare.split('.').all(|part| {
            !part.is_empty()
                && part.bytes().enumerate().all(|(i, b)| {
                    b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit())
                })
        }))
    .then(|| value.to_owned())
}
fn scope(mut node: Node<'_>, field: bool) -> Option<std::ops::Range<usize>> {
    while let Some(parent) = node.parent() {
        let matches = if field {
            matches!(
                parent.kind(),
                "class_declaration" | "struct_declaration" | "record_declaration"
            )
        } else {
            matches!(
                parent.kind(),
                "block"
                    | "method_declaration"
                    | "constructor_declaration"
                    | "local_function_statement"
                    | "lambda_expression"
                    | "anonymous_method_expression"
                    | "for_statement"
                    | "foreach_statement"
                    | "using_statement"
            )
        };
        if matches {
            return Some(parent.byte_range());
        }
        node = parent;
    }
    None
}

/// One parse and one tree walk; calls examine only bindings of their receiver.
/// Unknown declarations shadow fields too. Unsupported receivers remain pending.
pub fn analyze(source: &str) -> Option<Vec<Call>> {
    if source.len() > 4 * 1024 * 1024 {
        return None;
    }
    let mut parser = Parser::new();
    parser
        .set_language(&super::langs::grammar_for("csharp")?)
        .ok()?;
    let tree = parser.parse(source, None)?;
    if tree.root_node().has_error() {
        return None;
    }
    let mut pending = vec![(tree.root_node(), false, None)];
    let mut bindings: HashMap<String, Vec<Binding>> = HashMap::new();
    let mut calls = Vec::new();
    let mut visited = 0;
    while let Some((node, in_lambda, class_scope)) = pending.pop() {
        visited += 1;
        if visited > 200_000 {
            return None;
        }
        if node.kind() == "invocation_expression" && !in_lambda {
            calls.push((node, class_scope));
        }
        let declaration = match node.kind() {
            "variable_declarator" => {
                let parent = node.parent()?;
                let field = parent
                    .parent()
                    .is_some_and(|p| p.kind() == "field_declaration");
                let ty_node = parent.child_by_field_name("type");
                let mut ty = ty_node.and_then(|n| simple_type(text(n, source)));
                if ty_node.is_some_and(|n| text(n, source) == "var") {
                    let mut cursor = node.walk();
                    ty = node
                        .named_children(&mut cursor)
                        .find(|n| n.kind() == "object_creation_expression")
                        .and_then(|n| n.child_by_field_name("type"))
                        .and_then(|n| simple_type(text(n, source)));
                }
                node.child_by_field_name("name")
                    .map(|name| (name, ty, field))
            }
            "parameter" | "property_declaration" | "declaration_pattern" => {
                let field = node.kind() == "property_declaration";
                node.child_by_field_name("name").map(|name| {
                    (
                        name,
                        node.child_by_field_name("type")
                            .and_then(|n| simple_type(text(n, source))),
                        field,
                    )
                })
            }
            "implicit_parameter" => Some((node, None, false)),
            "foreach_statement" => node
                .child_by_field_name("left")
                .map(|name| (name, None, false)),
            "catch_declaration" => node
                .child_by_field_name("name")
                .map(|name| (name, None, false)),
            "single_variable_designation" => Some((node, None, false)),
            _ => None,
        };
        if let Some((name, ty, field)) = declaration {
            if let Some(scope) = scope(node, field) {
                bindings
                    .entry(text(name, source).to_owned())
                    .or_default()
                    .push(Binding {
                        ty,
                        scope,
                        declaration: node.start_byte(),
                        field,
                    });
            }
        }
        let mut cursor = node.walk();
        let in_lambda = in_lambda
            || matches!(
                node.kind(),
                "lambda_expression" | "anonymous_method_expression"
            );
        let class_scope = if matches!(
            node.kind(),
            "class_declaration" | "struct_declaration" | "record_declaration"
        ) {
            Some((node.start_byte(), node.end_byte()))
        } else {
            class_scope
        };
        pending.extend(
            node.named_children(&mut cursor)
                .map(|child| (child, in_lambda, class_scope)),
        );
    }
    let bindings = bindings
        .into_iter()
        .map(|(name, bindings)| (name, BindingIndex::new(bindings)))
        .collect::<HashMap<_, _>>();
    calls.sort_unstable_by_key(|(node, _)| node.start_byte());
    let mut positions = source.char_indices().peekable();
    let mut column = 0;
    let mut line = 1;
    let mut result = Vec::new();
    for (call, class_scope) in calls {
        // Calls arrive in byte order, including nested calls. Every preceding
        // character contributes to UTF-16 columns once, even on a long line.
        while positions
            .peek()
            .is_some_and(|(offset, _)| *offset < call.start_byte())
        {
            let (_, ch) = positions.next()?;
            if ch == '\n' {
                line += 1;
                column = 0;
            } else {
                column += ch.len_utf16();
            }
        }
        let Some(function) = call.child_by_field_name("function") else {
            continue;
        };
        if function.kind() != "member_access_expression" {
            continue;
        }
        let Some(receiver) = function.child_by_field_name("expression") else {
            continue;
        };
        let Some(method) = function.child_by_field_name("name") else {
            continue;
        };
        if method.kind() != "identifier" {
            continue;
        }
        let (name, explicit_this) = if receiver.kind() == "identifier" {
            (text(receiver, source), false)
        } else if receiver.kind() == "member_access_expression"
            && receiver
                .child_by_field_name("expression")
                .is_some_and(|n| text(n, source) == "this")
        {
            let Some(name) = receiver.child_by_field_name("name") else {
                continue;
            };
            (text(name, source), true)
        } else {
            continue;
        };
        let Some(available) = bindings.get(name) else {
            continue;
        };
        let Some(binding) = available.select(call.start_byte(), explicit_this, class_scope) else {
            continue;
        };
        // Same-scope duplicate declarations or a later local declaration must
        // not fall through to a field with the same name.
        if !binding.field && binding.declaration > call.start_byte() {
            continue;
        }
        let Some(ty) = &binding.ty else {
            continue;
        };
        if result.len() >= 20_000 {
            return None;
        }
        result.push(Call {
            name: text(function, source).to_owned(),
            type_name: ty.clone(),
            method: text(method, source).to_owned(),
            line,
            column,
        });
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_receiver_names_use_logarithmic_interval_lookup() {
        let index = BindingIndex::new(
            (0..1000)
                .map(|i| Binding {
                    ty: Some(format!("Type{i}")),
                    scope: i * 10..i * 10 + 5,
                    declaration: i * 10,
                    field: false,
                })
                .collect(),
        );
        for i in 0..1000 {
            let binding = index.select(i * 10 + 1, false, None).unwrap();
            assert_eq!(binding.ty.as_deref(), Some(format!("Type{i}").as_str()));
            assert!(
                index.visited.get() < 40,
                "visited {} bindings",
                index.visited.get()
            );
        }
    }
    #[test]
    fn one_thousand_same_name_parameters_keep_long_line_utf16_columns() {
        let mut source = String::from("class View { ");
        let mut column = source.len();
        let mut expected = Vec::new();
        for i in 0..1000 {
            let prefix = format!("void Run{i}(Demo.Controller receiver) {{ /*😀*/ ");
            column += prefix.encode_utf16().count();
            source.push_str(&prefix);
            expected.push(column);
            let suffix = "receiver.Execute(); } ";
            column += suffix.len();
            source.push_str(suffix);
        }
        source.push('}');
        let calls = analyze(&source).unwrap();
        assert_eq!(calls.len(), 1000);
        for (call, column) in calls.iter().zip(expected) {
            assert_eq!(call.type_name, "Demo.Controller");
            assert_eq!((call.line, call.column), (1, column));
        }
    }
    #[test]
    fn nested_classes_do_not_capture_outer_instance_fields() {
        let calls=analyze(concat!(
            "class Outer { Demo.OuterController receiver; ",
            "class NoField { void Run() { receiver.Execute(); this.receiver.Execute(); } } ",
            "class Inner { Demo.InnerController receiver; void Run() { receiver.Execute(); this.receiver.Execute(); } } }"
        )).unwrap();
        assert_eq!(calls.len(), 2);
        assert!(
            calls
                .iter()
                .all(|call| call.type_name == "Demo.InnerController")
        );
    }
}

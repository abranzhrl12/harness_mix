//! JavaScript and TypeScript declaration symbol collection using Oxc AST.
//! Extracts top-level and class/interface/enum member symbols with signatures and UTF-16 symbolIds.

use crate::adapters::parsers::model::Symbol;
use harness_domain::{Failure, execution::OperationContext, hash, results::prefix};
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    BindingPattern, ClassElement, Declaration, ExportDefaultDeclarationKind, Statement,
    TSInterfaceDeclaration, TSTypeAliasDeclaration,
};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType, Span};

struct TsCollector<'a> {
    source: &'a str,
    digest: &'a str,
    context: &'a OperationContext,
    symbols: Vec<Symbol>,
}

impl<'a> TsCollector<'a> {
    fn new(source: &'a str, digest: &'a str, context: &'a OperationContext) -> Self {
        Self {
            source,
            digest,
            context,
            symbols: Vec::new(),
        }
    }

    fn record(
        &mut self,
        name: &str,
        container: &str,
        kind: &'static str,
        span: Span,
        body_start: Option<u32>,
    ) -> Result<String, Failure> {
        self.context.check()?;
        if self.symbols.len() >= 1000 || name.len() > 1024 || container.len() > 4096 {
            return Err(("PARSER_LIMIT", "Symbol count or identifier budget exceeded"));
        }

        let start_u = span.start as usize;
        let end_u = span.end as usize;
        if start_u >= end_u || self.source.get(start_u..end_u).is_none() {
            return Err(("RANGE", "Parser offsets incompatible with source"));
        }

        let full_text = &self.source[start_u..end_u];
        let sig_end = body_start.map(|b| (b as usize).min(end_u)).unwrap_or(end_u);
        let sig_slice = &self.source[start_u..sig_end];
        let compact = sig_slice.split_whitespace().collect::<Vec<_>>().join(" ");
        let signature = if body_start.is_some() {
            format!("{} …", prefix(&compact, 398))
        } else {
            prefix(&compact, 400).to_owned()
        };

        // Extract preceding doc comment (e.g. /** ... */)
        let doc = self.extract_preceding_doc(start_u);

        // Calculate UTF-16 offset-based ID digest matching the Node harness convention
        let from = self.source[..start_u].encode_utf16().count();
        let to = from + full_text.encode_utf16().count();
        let id = hash(format!("{}:{from}:{to}", self.digest).as_bytes())[..24].to_owned();

        let start_line = self.source[..start_u]
            .bytes()
            .filter(|b| *b == b'\n')
            .count()
            + 1;
        let end_line = self.source[..end_u].bytes().filter(|b| *b == b'\n').count()
            + usize::from(!self.source[..end_u].ends_with('\n'));

        let child = if container.is_empty() {
            name.to_string()
        } else {
            format!("{container}::{name}")
        };

        self.symbols.push(Symbol {
            name: name.to_string(),
            container: container.to_string(),
            kind,
            start: start_line,
            end: end_line,
            signature,
            attributes: Vec::new(),
            doc: prefix(&doc, 600).into(),
            id,
            bytes: start_u..end_u,
        });

        Ok(child)
    }

    fn extract_preceding_doc(&self, start_pos: usize) -> String {
        let before = self.source[..start_pos].trim_end();
        if before.ends_with("*/")
            && let Some(open) = before.rfind("/*")
        {
            let comment = &before[open + 2..before.len() - 2];
            return comment
                .lines()
                .map(|l| l.trim().trim_start_matches('*').trim())
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
        }
        String::new()
    }

    fn visit_statement(&mut self, stmt: &Statement, container: &str) -> Result<(), Failure> {
        self.context.check()?;
        match stmt {
            Statement::FunctionDeclaration(func) => {
                if let Some(ident) = &func.id {
                    let body_start = func.body.as_ref().map(|b| b.span.start);
                    self.record(
                        &ident.name,
                        container,
                        "function_declaration",
                        func.span,
                        body_start,
                    )?;
                }
            }
            Statement::ClassDeclaration(cls) => {
                if let Some(ident) = &cls.id {
                    let body_start = Some(cls.body.span.start);
                    let class_container = self.record(
                        &ident.name,
                        container,
                        "class_declaration",
                        cls.span,
                        body_start,
                    )?;
                    for element in &cls.body.body {
                        self.visit_class_element(element, &class_container)?;
                    }
                }
            }
            Statement::VariableDeclaration(var_decl) => {
                for decl in &var_decl.declarations {
                    if let BindingPattern::BindingIdentifier(ident) = &decl.id {
                        let kind = if decl.init.as_ref().is_some_and(|init| {
                            matches!(
                                init,
                                oxc_ast::ast::Expression::ArrowFunctionExpression(_)
                                    | oxc_ast::ast::Expression::FunctionExpression(_)
                            )
                        }) {
                            "function_definition"
                        } else {
                            "variable_declarator"
                        };
                        let body_start = decl.init.as_ref().map(|e| e.span().start);
                        self.record(&ident.name, container, kind, decl.span, body_start)?;
                    }
                }
            }
            Statement::ExportDeclaration(export_decl) => {
                self.visit_declaration(&export_decl.declaration, container)?;
            }
            Statement::ExportDefaultDeclaration(export_default) => {
                match &export_default.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
                        let name = func
                            .id
                            .as_ref()
                            .map(|i| i.name.as_str())
                            .unwrap_or("default");
                        let body_start = func.body.as_ref().map(|b| b.span.start);
                        self.record(
                            name,
                            container,
                            "function_declaration",
                            func.span,
                            body_start,
                        )?;
                    }
                    ExportDefaultDeclarationKind::ClassDeclaration(cls) => {
                        let name = cls
                            .id
                            .as_ref()
                            .map(|i| i.name.as_str())
                            .unwrap_or("default");
                        let body_start = Some(cls.body.span.start);
                        let class_container = self.record(
                            name,
                            container,
                            "class_declaration",
                            cls.span,
                            body_start,
                        )?;
                        for element in &cls.body.body {
                            self.visit_class_element(element, &class_container)?;
                        }
                    }
                    ExportDefaultDeclarationKind::TSInterfaceDeclaration(iface) => {
                        self.visit_interface(iface, container)?;
                    }
                    _ => {}
                }
            }
            Statement::TSInterfaceDeclaration(iface) => {
                self.visit_interface(iface, container)?;
            }
            Statement::TSTypeAliasDeclaration(alias) => {
                self.visit_type_alias(alias, container)?;
            }
            Statement::TSEnumDeclaration(enum_decl) => {
                let name = &enum_decl.id.name;
                let enum_container = self.record(
                    name,
                    container,
                    "enum_declaration",
                    enum_decl.span,
                    Some(enum_decl.span.start),
                )?;
                for member in &enum_decl.body.members {
                    let id_span = member.id.span();
                    let member_name = &self.source[id_span.start as usize..id_span.end as usize];
                    self.record(member_name, &enum_container, "enum_item", member.span, None)?;
                }
            }
            Statement::TSNamespaceDeclaration(module_decl) => {
                let mod_container = self.record(
                    module_decl.id.name.as_str(),
                    container,
                    "internal_module",
                    module_decl.span,
                    None,
                )?;
                if let oxc_ast::ast::TSNamespaceDeclarationBody::TSModuleBlock(block) =
                    &module_decl.body
                {
                    for inner in &block.body {
                        self.visit_statement(inner, &mod_container)?;
                    }
                }
            }
            Statement::TSExternalModuleDeclaration(module_decl) => {
                if let Some(block) = &module_decl.body {
                    let name = module_decl.id.value.as_str().unwrap_or("");
                    let mod_container =
                        self.record(name, container, "internal_module", module_decl.span, None)?;
                    for inner in &block.body {
                        self.visit_statement(inner, &mod_container)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn visit_declaration(&mut self, decl: &Declaration, container: &str) -> Result<(), Failure> {
        match decl {
            Declaration::FunctionDeclaration(func) => {
                if let Some(ident) = &func.id {
                    let body_start = func.body.as_ref().map(|b| b.span.start);
                    self.record(
                        &ident.name,
                        container,
                        "function_declaration",
                        func.span,
                        body_start,
                    )?;
                }
            }
            Declaration::ClassDeclaration(cls) => {
                if let Some(ident) = &cls.id {
                    let body_start = Some(cls.body.span.start);
                    let class_container = self.record(
                        &ident.name,
                        container,
                        "class_declaration",
                        cls.span,
                        body_start,
                    )?;
                    for element in &cls.body.body {
                        self.visit_class_element(element, &class_container)?;
                    }
                }
            }
            Declaration::VariableDeclaration(var_decl) => {
                for d in &var_decl.declarations {
                    if let BindingPattern::BindingIdentifier(ident) = &d.id {
                        let kind = if d.init.as_ref().is_some_and(|init| {
                            matches!(
                                init,
                                oxc_ast::ast::Expression::ArrowFunctionExpression(_)
                                    | oxc_ast::ast::Expression::FunctionExpression(_)
                            )
                        }) {
                            "function_definition"
                        } else {
                            "variable_declarator"
                        };
                        let body_start = d.init.as_ref().map(|e| e.span().start);
                        self.record(&ident.name, container, kind, d.span, body_start)?;
                    }
                }
            }
            Declaration::TSInterfaceDeclaration(iface) => {
                self.visit_interface(iface, container)?;
            }
            Declaration::TSTypeAliasDeclaration(alias) => {
                self.visit_type_alias(alias, container)?;
            }
            Declaration::TSEnumDeclaration(enum_decl) => {
                let name = &enum_decl.id.name;
                let enum_container = self.record(
                    name,
                    container,
                    "enum_declaration",
                    enum_decl.span,
                    Some(enum_decl.span.start),
                )?;
                for member in &enum_decl.body.members {
                    let id_span = member.id.span();
                    let member_name = &self.source[id_span.start as usize..id_span.end as usize];
                    self.record(member_name, &enum_container, "enum_item", member.span, None)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn visit_interface(
        &mut self,
        iface: &TSInterfaceDeclaration,
        container: &str,
    ) -> Result<(), Failure> {
        let name = &iface.id.name;
        let iface_container = self.record(
            name,
            container,
            "interface_declaration",
            iface.span,
            Some(iface.body.span.start),
        )?;
        for member in &iface.body.body {
            match member {
                oxc_ast::ast::TSSignature::TSPropertySignature(prop) => {
                    let prop_name = match &prop.key {
                        oxc_ast::ast::PropertyKey::StaticIdentifier(id) => id.name.as_str(),
                        _ => continue,
                    };
                    self.record(
                        prop_name,
                        &iface_container,
                        "property_declaration",
                        prop.span,
                        None,
                    )?;
                }
                oxc_ast::ast::TSSignature::TSMethodSignature(method) => {
                    let method_name = match &method.key {
                        oxc_ast::ast::PropertyKey::StaticIdentifier(id) => id.name.as_str(),
                        _ => continue,
                    };
                    self.record(
                        method_name,
                        &iface_container,
                        "method_signature",
                        method.span,
                        None,
                    )?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn visit_type_alias(
        &mut self,
        alias: &TSTypeAliasDeclaration,
        container: &str,
    ) -> Result<(), Failure> {
        self.record(
            &alias.id.name,
            container,
            "type_alias_declaration",
            alias.span,
            None,
        )?;
        Ok(())
    }

    fn visit_class_element(
        &mut self,
        element: &ClassElement,
        container: &str,
    ) -> Result<(), Failure> {
        match element {
            ClassElement::MethodDefinition(method) => {
                let name = match &method.key {
                    oxc_ast::ast::PropertyKey::StaticIdentifier(id) => id.name.as_str(),
                    _ => return Ok(()),
                };
                let body_start = method.value.body.as_ref().map(|b| b.span.start);
                self.record(
                    name,
                    container,
                    "method_definition",
                    method.span,
                    body_start,
                )?;
            }
            ClassElement::PropertyDefinition(prop) => {
                let name = match &prop.key {
                    oxc_ast::ast::PropertyKey::StaticIdentifier(id) => id.name.as_str(),
                    _ => return Ok(()),
                };
                let body_start = prop.value.as_ref().map(|e| e.span().start);
                self.record(
                    name,
                    container,
                    "property_declaration",
                    prop.span,
                    body_start,
                )?;
            }
            _ => {}
        }
        Ok(())
    }
}

pub(in crate::adapters) fn collect(
    path: &str,
    source: &str,
    digest: &str,
    context: &OperationContext,
) -> Result<Vec<Symbol>, Failure> {
    context.check()?;
    if source.len() > 512 * 1024 {
        return Err(("PARSER_LIMIT", "JS/TS source exceeds 512 KiB limit"));
    }

    // Match syntax validation for extensions with non-canonical casing while
    // keeping the original source untouched for span and byte-offset mapping.
    let normalized_path = path.to_ascii_lowercase();
    let source_type = SourceType::from_path(normalized_path.as_str()).unwrap_or_default();
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();

    context.check()?;
    if parsed.diagnostics.has_errors() {
        return Err(("SYNTAX_ERROR", "JS/TS syntax error"));
    }

    let mut collector = TsCollector::new(source, digest, context);
    for stmt in &parsed.program.body {
        collector.visit_statement(stmt, "")?;
    }

    Ok(collector.symbols)
}

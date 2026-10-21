//! Bounded lexical proof for named value imports used as bare Vue script calls.
use super::analyze as analyze_calls;
use crate::adapters::parsers::vue::structure::{self, Block};
use harness_domain::{Failure, execution::OperationContext};
use oxc_allocator::Allocator;
use oxc_ast::ast::Statement;
use oxc_parser::Parser;
use oxc_span::SourceType;
use std::collections::BTreeMap;

const INVALID: Failure = (
    "SYNC_SEMANTIC",
    "Cannot establish unambiguous Vue named-import call bindings",
);
const LIMIT: Failure = (
    "SYNC_LIMIT",
    "Vue semantic analysis exceeded its bounded graph budget",
);
const MAX_BINDINGS: usize = 20_000;
const MAX_RESERVED: usize = 16 * 1024 * 1024;

#[derive(Default)]
pub(in crate::adapters) struct Facts {
    bindings: BTreeMap<(String, i64, i64), (String, String, String)>,
    pub(in crate::adapters) reserved: usize,
}

impl Facts {
    pub(in crate::adapters) fn binding(
        &self,
        name: &str,
        line: i64,
        col: i64,
    ) -> Option<(String, String, String)> {
        self.bindings.get(&(name.to_owned(), line, col)).cloned()
    }
}

pub(in crate::adapters) fn analyze(
    source: &str,
    path: &str,
    context: &OperationContext,
) -> Result<Facts, Failure> {
    context.check()?;
    if source.len() > 512 * 1024 {
        return Err(LIMIT);
    }
    let digest = harness_domain::hash(source.as_bytes());
    let document = structure::parse(source, &digest, context)?;
    let mut facts = Facts::default();

    for block in document
        .blocks
        .iter()
        .filter(|block| block.kind == "script")
    {
        context.check()?;
        if structure::attr(&block.attrs, "src").is_some() {
            continue;
        }
        let Some(language) = script_language(block) else {
            continue;
        };
        let projection =
            crate::adapters::index_extract::framework::project(source, block.body.clone())?;
        let parser_path = format!("{path}.{}", language_extension(language));
        let calls = analyze_calls(&projection, &parser_path, context)?;
        let imports = import_bindings(&projection, &parser_path, context)?;
        facts.reserved = facts.reserved.checked_add(calls.reserved).ok_or(LIMIT)?;
        if facts.reserved > MAX_RESERVED {
            return Err(LIMIT);
        }

        for (name, line, col) in calls.sites {
            context.check()?;
            let Some((module, imported)) = imports.get(&name) else {
                // Calls proven to bind to an import but whose import form is
                // outside the named-value subset remain unresolved.
                continue;
            };
            let row = (name, line, col);
            let binding = (module.clone(), imported.clone(), language.to_owned());
            facts.reserved = facts
                .reserved
                .checked_add(row.0.len() + module.len() + imported.len() + language.len() + 48)
                .ok_or(LIMIT)?;
            if facts.bindings.len() >= MAX_BINDINGS || facts.reserved > MAX_RESERVED {
                return Err(LIMIT);
            }
            if facts.bindings.insert(row, binding).is_some() {
                return Err(INVALID);
            }
        }
    }
    Ok(facts)
}

fn script_language(block: &Block) -> Option<&'static str> {
    match structure::attr(&block.attrs, "lang").unwrap_or("js") {
        "js" | "javascript" => Some("javascript"),
        "ts" | "typescript" => Some("typescript"),
        "jsx" => Some("jsx"),
        "tsx" => Some("tsx"),
        _ => None,
    }
}

fn language_extension(language: &str) -> &'static str {
    match language {
        "javascript" => "js",
        "typescript" => "ts",
        "jsx" => "jsx",
        "tsx" => "tsx",
        _ => unreachable!("script language is selected by script_language"),
    }
}

fn source_type(path: &str) -> Result<SourceType, Failure> {
    SourceType::from_path(path).map_err(|_| INVALID)
}

fn import_bindings(
    projection: &str,
    parser_path: &str,
    context: &OperationContext,
) -> Result<BTreeMap<String, (String, String)>, Failure> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, projection, source_type(parser_path)?).parse();
    context.check()?;
    if !parsed.errors.is_empty() {
        return Err(INVALID);
    }
    let mut imports = BTreeMap::new();
    for statement in &parsed.program.body {
        context.check()?;
        let Statement::ImportDeclaration(declaration) = statement else {
            continue;
        };
        let start = declaration.span.start as usize;
        let end = declaration.span.end as usize;
        let signature = projection.get(start..end).ok_or(INVALID)?;
        let Some((module, names)) = codegraph_native::named_imports(signature) else {
            // Defaults, namespaces, type-only forms, and import attributes are
            // deliberately outside this resolver's proof.
            continue;
        };
        for (imported, local) in names {
            context.check()?;
            if imports.insert(local, (module.clone(), imported)).is_some() {
                return Err(INVALID);
            }
        }
    }
    Ok(imports)
}

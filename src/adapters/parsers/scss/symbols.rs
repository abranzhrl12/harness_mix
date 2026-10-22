//! SCSS symbols extracted from a real parse tree.
//!
//! Every construct keeps the exact source range it came from, so `outline` and
//! `read` return the original bytes instead of a re-serialized approximation.
use crate::adapters::parsers::model::Symbol;
use harness_domain::{Failure, execution::OperationContext, hash};
use raffia::{
    Parser, Syntax,
    ast::{
        AtRulePrelude, FunctionName, InterpolableIdent, InterpolableStr, SassModuleMemberName,
        Statement,
    },
    pos::Span,
};

const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_SYMBOLS: usize = 5000;
const MAX_DEPTH: usize = 64;
const MAX_NAME: usize = 1024;
const MAX_CONTAINER: usize = 4096;

/// Collects the SCSS symbols of `source`.
///
/// A file that does not parse is reported as a syntax failure instead of a
/// partial inventory: a half-read stylesheet is worse than an explicit error.
pub(in crate::adapters) fn collect(
    source: &str,
    digest: &str,
    context: &OperationContext,
) -> Result<Vec<Symbol>, Failure> {
    if source.len() > MAX_BYTES {
        return Err(("PARSER_LIMIT", "SCSS source budget exceeded"));
    }
    context.check()?;
    let mut parser = Parser::new(source, Syntax::Scss);
    let stylesheet = parser.parse::<raffia::ast::Stylesheet>().map_err(|_| {
        (
            "SYNTAX_ERROR",
            "SCSS source is not syntactically valid",
        )
    })?;
    let mut collector = Collector {
        source,
        digest,
        lines: LineIndex::new(source),
        symbols: Vec::new(),
        context,
    };
    collector.walk(&stylesheet.statements, "", 0)?;
    collector.symbols.sort_by_key(|item| item.bytes.start);
    Ok(collector.symbols)
}

struct Collector<'a> {
    source: &'a str,
    digest: &'a str,
    lines: LineIndex,
    symbols: Vec<Symbol>,
    context: &'a OperationContext,
}

impl Collector<'_> {
    fn walk(
        &mut self,
        statements: &[Statement<'_>],
        container: &str,
        depth: usize,
    ) -> Result<(), Failure> {
        if depth > MAX_DEPTH {
            return Err(("PARSER_LIMIT", "SCSS nesting budget exceeded"));
        }
        for statement in statements {
            self.context.check()?;
            if self.symbols.len() >= MAX_SYMBOLS {
                return Err(("PARSER_LIMIT", "SCSS symbol budget exceeded"));
            }
            match statement {
                Statement::QualifiedRule(rule) => {
                    let name = collapse(self.text(&rule.selector.span));
                    self.push("scss-rule", &name, container, &rule.span, signature_rule(&name));
                    let nested = join(container, &name);
                    self.walk(&rule.block.statements, &nested, depth + 1)?;
                }
                Statement::SassVariableDeclaration(declaration) => {
                    let name = format!("${}", declaration.name.name.name);
                    self.push(
                        "scss-variable",
                        &name,
                        container,
                        &declaration.span,
                        name.clone(),
                    );
                }
                Statement::SassIfAtRule(rule) => {
                    let name = "@if".to_owned();
                    self.push(
                        "scss-control",
                        &name,
                        container,
                        &rule.span,
                        name.clone(),
                    );
                    let nested = join(container, &name);
                    self.walk(&rule.if_clause.block.statements, &nested, depth + 1)?;
                    for clause in &rule.else_if_clauses {
                        self.walk(&clause.block.statements, &nested, depth + 1)?;
                    }
                    if let Some(else_clause) = &rule.else_clause {
                        self.walk(&else_clause.statements, &nested, depth + 1)?;
                    }
                }
                Statement::AtRule(rule) => self.at_rule(rule, container, depth)?,
                Statement::KeyframeBlock(_) | Statement::UnknownSassAtRule(_) => {}
                _ => {}
            }
        }
        Ok(())
    }

    fn at_rule(
        &mut self,
        rule: &raffia::ast::AtRule<'_>,
        container: &str,
        depth: usize,
    ) -> Result<(), Failure> {
        let keyword = rule.name.name.clone();
        let (kind, name) = match rule.prelude.as_ref() {
            Some(AtRulePrelude::SassMixin(mixin)) => {
                ("scss-mixin", mixin.name.name.to_string())
            }
            Some(AtRulePrelude::SassFunction(function)) => {
                ("scss-function", function.name.name.to_string())
            }
            Some(AtRulePrelude::SassInclude(include)) => {
                ("scss-include", self.function_name(&include.name))
            }
            Some(AtRulePrelude::SassUse(use_rule)) => {
                ("scss-use", self.interpolable_str(&use_rule.path))
            }
            Some(AtRulePrelude::SassForward(forward)) => {
                ("scss-forward", self.interpolable_str(&forward.path))
            }
            Some(AtRulePrelude::SassExtend(extend)) => {
                ("scss-extend", collapse(self.text(&extend.selectors.span)))
            }
            Some(AtRulePrelude::SassEach(_)) => ("scss-control", format!("@{keyword}")),
            Some(AtRulePrelude::SassFor(_)) => ("scss-control", format!("@{keyword}")),
            Some(AtRulePrelude::Media(media)) => {
                ("scss-at-rule", grouping("@media", self.text(&media.span)))
            }
            Some(AtRulePrelude::Supports(supports)) => {
                ("scss-at-rule", grouping("@supports", self.text(&supports.span)))
            }
            Some(AtRulePrelude::Container(container)) => {
                ("scss-at-rule", grouping("@container", self.text(&container.span)))
            }
            Some(AtRulePrelude::Layer(layer)) => {
                ("scss-at-rule", grouping("@layer", self.text(&layer.span)))
            }
            _ => ("scss-at-rule", format!("@{keyword}")),
        };
        self.push(kind, &name, container, &rule.span, name.clone());
        if let Some(block) = &rule.block {
            let nested = join(container, &name);
            self.walk(&block.statements, &nested, depth + 1)?;
        }
        Ok(())
    }

    fn function_name(&self, name: &FunctionName<'_>) -> String {
        match name {
            FunctionName::Ident(ident) => self.interpolable_ident(ident),
            FunctionName::SassQualifiedName(qualified) => {
                let member = match &qualified.member {
                    SassModuleMemberName::Ident(ident) => ident.name.to_string(),
                    SassModuleMemberName::Variable(variable) => {
                        format!("${}", variable.name.name)
                    }
                };
                format!("{}.{member}", qualified.module.name)
            }
            _ => String::new(),
        }
    }

    fn interpolable_ident(&self, ident: &InterpolableIdent<'_>) -> String {
        match ident {
            InterpolableIdent::Literal(literal) => literal.name.to_string(),
            InterpolableIdent::SassInterpolated(interpolated) => {
                collapse(self.text(&interpolated.span))
            }
            InterpolableIdent::LessInterpolated(interpolated) => {
                collapse(self.text(&interpolated.span))
            }
        }
    }

    fn interpolable_str(&self, value: &InterpolableStr<'_>) -> String {
        match value {
            InterpolableStr::Literal(literal) => literal.value.to_string(),
            InterpolableStr::SassInterpolated(interpolated) => {
                collapse(self.text(&interpolated.span))
            }
            InterpolableStr::LessInterpolated(interpolated) => {
                collapse(self.text(&interpolated.span))
            }
        }
    }

    fn text(&self, span: &Span) -> &str {
        self.source
            .get(span.start..span.end)
            .unwrap_or_default()
    }

    fn push(
        &mut self,
        kind: &'static str,
        name: &str,
        container: &str,
        span: &Span,
        signature: String,
    ) {
        let from = span.start.min(self.source.len());
        let to = span.end.min(self.source.len()).max(from);
        let name = truncate(name, MAX_NAME);
        let container = truncate(container, MAX_CONTAINER);
        self.symbols.push(Symbol {
            name,
            container,
            kind,
            start: self.lines.line_of(from),
            end: self.lines.line_of(to),
            signature: truncate(&signature, 512),
            attributes: Vec::new(),
            doc: String::new(),
            id: hash(format!("{}:{from}:{to}:scss", self.digest).as_bytes())[..24].into(),
            bytes: from..to,
        });
    }
}

/// One-based line for a byte offset, resolved with a single sorted index.
struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(source: &str) -> Self {
        let mut starts = vec![0usize];
        starts.extend(
            source
                .bytes()
                .enumerate()
                .filter(|(_, byte)| *byte == b'\n')
                .map(|(at, _)| at + 1),
        );
        Self { starts }
    }

    fn line_of(&self, offset: usize) -> usize {
        self.starts.partition_point(|start| *start <= offset)
    }
}

fn signature_rule(name: &str) -> String {
    format!("{name} {{ … }}")
}

/// Collapses whitespace runs so a selector stays a single readable line.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Builds the display name of a grouping at-rule.
fn grouping(keyword: &str, prelude: &str) -> String {
    let detail = collapse(prelude);
    if detail.is_empty() {
        keyword.to_owned()
    } else {
        format!("{keyword} {detail}")
    }
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut out = text.chars().take(limit).collect::<String>();
    out.push('…');
    out
}

fn join(container: &str, name: &str) -> String {
    if container.is_empty() {
        name.to_owned()
    } else {
        format!("{container} / {name}")
    }
}

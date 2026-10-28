use super::super::{
    project::{Failure, Project},
    symbol_graph,
};
use super::matcher_cache::{self, Options};
use harness_domain::{
    contracts::{Kind, SearchArgs},
    results::utf16_len,
};
use std::path::PathBuf;

pub(super) struct SearchPlan<'a> {
    pub(super) file_size: bool,
    /// Minified/bundled artifacts and package-output directories stay out of
    /// text scans by default: they exhaust the scan budget without informing
    /// source queries. kind=file_size keeps its dedicated size ranking and
    /// kind=files can still enumerate them by name.
    pub(super) exclude_minified: bool,
    pub(super) syntax: bool,
    pub(super) context_lines: usize,
    pub(super) selected: Option<(&'static str, Option<&'a str>)>,
    pub(super) globs: Option<globset::GlobSet>,
    pub(super) matcher: Option<grep_regex::RegexMatcher>,
    pub(super) term_retrieval: bool,
    pub(super) structural: Option<super::structural::StructuralSearch>,
    pub(super) min_lines: usize,
    pub(super) limit: usize,
    pub(super) explicit_scope: bool,
    pub(super) scope: &'a str,
    pub(super) root: PathBuf,
}

pub(super) fn build<'a>(
    project: &Project,
    args: &'a SearchArgs,
) -> Result<SearchPlan<'a>, Failure> {
    build_with_matcher(project, args, None)
}

pub(super) fn build_fallback<'a>(
    project: &Project,
    args: &'a SearchArgs,
    matcher: grep_regex::RegexMatcher,
) -> Result<SearchPlan<'a>, Failure> {
    build_with_matcher(project, args, Some(matcher))
}

fn build_with_matcher<'a>(
    project: &Project,
    args: &'a SearchArgs,
    fallback_matcher: Option<grep_regex::RegexMatcher>,
) -> Result<SearchPlan<'a>, Failure> {
    let file_size = matches!(args.kind, Kind::FileSize);
    let exclude_minified = file_size || matches!(args.kind, Kind::Text);
    let syntax = matches!(args.kind, Kind::Syntax);
    let term_retrieval = fallback_matcher.is_some();
    let context_lines = args
        .context_lines
        .unwrap_or(if term_retrieval { 2 } else { 0 });
    if context_lines > 20 || (args.context_lines.is_some() && !matches!(args.kind, Kind::Text)) {
        return Err((
            "ARGUMENT",
            "contextLines requires kind text and a value between 0 and 20",
        ));
    }
    let selected = if syntax {
        Some(selector(&args.query)?)
    } else {
        None
    };
    let globs = glob_matcher(&args.globs, &args.kind)?;
    if args.ast.is_some()
        && (!matches!(args.kind, Kind::Text)
            || args.regex.is_some()
            || args.case_sensitive.is_some())
    {
        return Err((
            "ARGUMENT",
            "ast requires kind text and cannot combine with regex or caseSensitive",
        ));
    }
    if !matches!(args.kind, Kind::Text) && (args.regex.is_some() || args.case_sensitive.is_some()) {
        return Err(("ARGUMENT", "regex and caseSensitive require kind text"));
    }
    let min_lines = args.min_lines.unwrap_or(500);
    if min_lines > 10000000
        || (!file_size && args.min_lines.is_some())
        || (file_size && !args.query.is_empty())
    {
        return Err((
            "ARGUMENT",
            "file_size requires minLines 0..10000000 and no query; minLines is invalid for other kinds",
        ));
    }
    if !file_size
        && !(matches!(args.kind, Kind::Files) && args.globs.is_some() && args.query.is_empty())
        && (args.query.is_empty()
            || utf16_len(&args.query) > 16000
            || args.query.contains('\0')
            || (args.ast.is_none() && args.query.contains(['\n', '\r'])))
    {
        return Err((
            "ARGUMENT",
            "query no vacía, sin NUL, máximo 16000 caracteres; saltos de línea solo con ast",
        ));
    }
    let limit = args.limit.unwrap_or(if file_size { 30 } else { 40 });
    if !(1..=if file_size { 100 } else { 200 }).contains(&limit) {
        return Err((
            "ARGUMENT",
            if file_size {
                "file_size limit debe estar entre 1 y 100"
            } else {
                "limit debe estar entre 1 y 200"
            },
        ));
    }
    // Reuse compiled matchers across requests. A line terminator lets
    // ripgrep optimize the matcher and rejects regexes requiring multiline input.
    let structural = args
        .ast
        .as_deref()
        .map(|language| super::structural::StructuralSearch::new(language, &args.query))
        .transpose()?;
    let matcher = if matches!(args.kind, Kind::Text) && structural.is_none() {
        Some(match fallback_matcher {
            Some(matcher) => matcher,
            None => matcher_cache::get(
                &args.query,
                Options {
                    literal: !args.regex.unwrap_or(false),
                    case_sensitive: args.case_sensitive.unwrap_or(true),
                    multi_line: true,
                },
            )
            .map_err(|_| {
                (
                    "ARGUMENT",
                    "invalid or unsupported text regex; use single-line Rust regex syntax",
                )
            })?,
        })
    } else {
        None
    };
    let explicit_scope = args
        .scope
        .as_deref()
        .is_some_and(|scope| !scope.is_empty() && scope != ".");
    let scope = args
        .scope
        .as_deref()
        .filter(|scope| !scope.is_empty())
        .unwrap_or(".");
    let root = project.target(scope)?;
    if root
        .file_name()
        .is_some_and(|name| scope_excluded(&name.to_string_lossy()))
    {
        return Err(("PATH", "scope excluido de búsquedas"));
    }
    Ok(SearchPlan {
        file_size,
        exclude_minified,
        syntax,
        context_lines,
        selected,
        globs,
        matcher,
        term_retrieval,
        structural,
        min_lines,
        limit,
        explicit_scope,
        scope,
        root,
    })
}

// A deliberately small declaration selector, not ast-grep pattern syntax.
// Kinds are normalized to one canonical vocabulary shared by the live parsers
// and the graph, so the same selector means the same declaration everywhere.
fn selector(query: &str) -> Result<(&'static str, Option<&str>), Failure> {
    let (kind, name) = query
        .split_once(':')
        .map_or((query, None), |(k, n)| (k, Some(n)));
    let canonical =
        symbol_graph::canonical_kind(kind).filter(|_| symbol_graph::SELECTOR_KINDS.contains(&kind));
    if canonical.is_none()
        || name.is_some_and(|n| n.is_empty() || n.contains(':') || n.chars().any(char::is_control))
    {
        return Err((
            "ARGUMENT",
            "Syntax selector requires declaration_kind[:exact name]; AST patterns are not supported",
        ));
    }
    Ok((canonical.expect("validated canonical kind"), name))
}

fn glob_matcher(
    globs: &Option<Vec<String>>,
    kind: &Kind,
) -> Result<Option<globset::GlobSet>, Failure> {
    let patterns = match globs {
        Some(patterns) => patterns,
        None => return Ok(None),
    };
    if !matches!(kind, Kind::Text | Kind::Files | Kind::FileSize) {
        return Err((
            "ARGUMENT",
            "globs solo es válido para kind text, files o file_size",
        ));
    }
    if patterns.is_empty() || patterns.len() > 64 {
        return Err(("ARGUMENT", "globs requiere entre 1 y 64 patrones"));
    }
    let mut builder = globset::GlobSetBuilder::new();
    for pattern in patterns {
        if pattern.is_empty()
            || utf16_len(pattern) > 1024
            || pattern.contains(['\0', '\n', '\r'])
            || pattern.chars().any(char::is_control)
        {
            return Err((
                "ARGUMENT",
                "cada patrón glob debe ser no vacío, sin control/NUL ni saltos de línea, máximo 1024 caracteres",
            ));
        }
        let glob = globset::GlobBuilder::new(pattern)
            .literal_separator(false)
            .build()
            .map_err(|_| ("ARGUMENT", "patrón glob inválido"))?;
        builder.add(glob);
    }
    let set = builder
        .build()
        .map_err(|_| ("ARGUMENT", "patrón glob inválido"))?;
    Ok(Some(set))
}

pub(super) fn scope_excluded(name: &str) -> bool {
    let name = name.to_lowercase();
    [".git", ".hg", ".svn", "node_modules", "target"].contains(&name.as_str()) || excluded(&name)
}

pub(super) fn excluded(name: &str) -> bool {
    let name = name.to_lowercase();
    [".git", ".hg", ".svn"].contains(&name.as_str())
        || name.starts_with(".env")
        || name.starts_with("credential")
        || name.starts_with("secret")
        || name.starts_with("mcp_config.json")
        || ["oauth_creds.json", "id_rsa", "id_ed25519"].contains(&name.as_str())
}

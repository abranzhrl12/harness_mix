use super::super::{
    project::{Failure, Project},
    symbol_graph,
};
use harness_domain::{
    contracts::{Kind, SearchArgs},
    results::utf16_len,
};
use std::path::PathBuf;

pub(super) struct SearchPlan<'a> {
    pub(super) file_size: bool,
    pub(super) syntax: bool,
    pub(super) context_lines: usize,
    pub(super) selected: Option<(&'static str, Option<&'a str>)>,
    pub(super) globs: Option<globset::GlobSet>,
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
    let file_size = matches!(args.kind, Kind::FileSize);
    let syntax = matches!(args.kind, Kind::Syntax);
    let context_lines = args.context_lines.unwrap_or(0);
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
            || args.query.contains(['\n', '\r', '\0']))
    {
        return Err((
            "ARGUMENT",
            "query literal no vacía, sin saltos de línea ni NUL, máximo 16000 caracteres",
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
        syntax,
        context_lines,
        selected,
        globs,
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

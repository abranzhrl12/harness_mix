use harness_domain::{Failure, contracts::IgnoreArgs};

pub(super) const MAX_PATTERNS: usize = 128;
pub(super) const MAX_FILE_BYTES: usize = 256 * 1024;
pub(super) const MAX_PATTERN_CHARS: usize = 1024;

pub(super) struct AddOutcome {
    pub(super) text: String,
    pub(super) added: Vec<String>,
    pub(super) entries: usize,
    pub(super) patterns: usize,
}

pub(super) struct RemoveOutcome {
    pub(super) text: String,
    pub(super) removed: Vec<String>,
    pub(super) entries: usize,
    pub(super) patterns: usize,
}

pub(super) fn normalize_scope(scope: Option<&str>) -> Result<String, Failure> {
    let scope = scope.unwrap_or(".");
    if scope.contains(['\n', '\r', '\0']) || scope.chars().any(char::is_control) {
        return Err(("ARGUMENT", "Ignore scope contains control characters"));
    }
    let scope = scope.trim();
    Ok(if scope.is_empty() || scope == "." {
        ".".to_owned()
    } else {
        scope.replace('\\', "/")
    })
}

pub(super) fn requested_patterns(args: &IgnoreArgs) -> Result<Vec<String>, Failure> {
    let patterns = args.patterns.as_deref().unwrap_or_default();
    if patterns.is_empty() {
        return Err((
            "ARGUMENT",
            "patterns requiere entre 1 y 128 patrones para add/remove",
        ));
    }
    if patterns.len() > MAX_PATTERNS {
        return Err(("LIMIT", "patterns admite como máximo 128 patrones"));
    }
    let mut result = Vec::with_capacity(patterns.len());
    for pattern in patterns {
        let pattern = pattern.trim();
        validate_pattern(pattern)?;
        if !result.iter().any(|existing: &String| existing == pattern) {
            result.push(pattern.to_owned());
        }
    }
    Ok(result)
}

pub(super) fn clean_reason(reason: Option<&str>) -> Result<Option<String>, Failure> {
    let Some(reason) = reason else {
        return Ok(None);
    };
    let reason = reason.trim();
    if reason.is_empty() {
        return Ok(None);
    }
    if reason.chars().count() > 200
        || reason.contains(['\n', '\r', '\0'])
        || reason.chars().any(char::is_control)
    {
        return Err((
            "ARGUMENT",
            "reason debe ser texto de una línea, máximo 200 caracteres",
        ));
    }
    Ok(Some(reason.replace('\r', " ")))
}

pub(super) fn add_patterns(
    text: &str,
    patterns: &[String],
    reason: Option<&str>,
) -> Result<AddOutcome, Failure> {
    let present: std::collections::BTreeSet<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect();
    let added: Vec<String> = patterns
        .iter()
        .filter(|pattern| !present.contains(*pattern))
        .cloned()
        .collect();
    let mut updated = text.to_owned();
    if !added.is_empty() {
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        if let Some(reason) = reason {
            updated.push_str("# ");
            updated.push_str(reason);
            updated.push('\n');
        }
        for pattern in &added {
            updated.push_str(pattern);
            updated.push('\n');
        }
    }
    let (entries, patterns) = entries_of(&updated);
    if patterns > MAX_PATTERNS {
        return Err((
            "LIMIT",
            "Ignore policy would exceed 128 patterns; remove obsolete entries first",
        ));
    }
    Ok(AddOutcome {
        text: updated,
        added,
        entries,
        patterns,
    })
}

pub(super) fn remove_patterns(text: &str, patterns: &[String]) -> RemoveOutcome {
    let wanted: std::collections::BTreeSet<&str> = patterns.iter().map(String::as_str).collect();
    let mut removed = Vec::new();
    let mut kept = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() && !trimmed.starts_with('#') && wanted.contains(trimmed) {
            removed.push(trimmed.to_owned());
        } else {
            kept.push(line);
        }
    }
    let mut updated = kept.join("\n");
    if !updated.is_empty() {
        updated.push('\n');
    }
    let (entries, patterns) = entries_of(&updated);
    RemoveOutcome {
        text: updated,
        removed,
        entries,
        patterns,
    }
}

pub(super) fn entries_of(text: &str) -> (usize, usize) {
    let entries = text
        .lines()
        .filter(|line| {
            let line = line.trim();
            !line.is_empty() && !line.starts_with('#')
        })
        .count();
    (entries, entries)
}

fn validate_pattern(pattern: &str) -> Result<(), Failure> {
    if pattern.is_empty()
        || pattern.chars().count() > MAX_PATTERN_CHARS
        || pattern.contains(['\n', '\r', '\0'])
        || pattern.chars().any(char::is_control)
    {
        return Err((
            "ARGUMENT",
            "cada patrón debe ser no vacío, sin control/NUL ni saltos de línea, máximo 1024 caracteres",
        ));
    }
    let body = pattern.strip_prefix('!').unwrap_or(pattern);
    if body.is_empty() {
        return Err(("ARGUMENT", "A negation marker requires a pattern"));
    }
    let normalized = body.replace('\\', "/");
    if normalized.starts_with('/') || body.contains(':') {
        return Err((
            "ARGUMENT",
            "los patrones deben ser relativos al directorio de .harnessignore, sin rutas absolutas",
        ));
    }
    if normalized
        .split('/')
        .any(|part| part == ".." || part == "." && normalized.contains('/'))
    {
        return Err(("ARGUMENT", "los patrones no pueden salir del scope con .."));
    }
    if normalized
        .split('/')
        .any(|part| [".git", ".hg", ".svn"].contains(&part.to_ascii_lowercase().as_str()))
    {
        return Err((
            "ARGUMENT",
            "Version-control metadata cannot be excluded or re-included by policy",
        ));
    }
    let lower = normalized.to_ascii_lowercase();
    let reinclude = pattern.starts_with('!');
    let sensitive = lower.starts_with(".env")
        || lower.starts_with("credential")
        || lower.starts_with("secret")
        || [
            "id_rsa",
            "id_ed25519",
            "oauth_creds.json",
            "mcp_config.json",
        ]
        .contains(&lower.as_str());
    if reinclude && sensitive {
        return Err((
            "PATH",
            "Secret and credential paths cannot be re-included by policy",
        ));
    }
    if matches!(
        pattern,
        "*" | "**" | "/*" | "/**" | "!*" | "!**" | "!/*" | "!/**"
    ) || normalized == "**"
    {
        return Err((
            "ARGUMENT",
            "A pattern that excludes the entire workspace is not allowed",
        ));
    }
    Ok(())
}

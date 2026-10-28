//! Fast, transparent per-file syntax and structural verification.
//!
//! When specific source files (such as `.js`, `.mjs`, `.cjs`, `.jsx`, `.ts`, `.tsx`,
//! `.mts`, `.cts`, `.cshtml`, `.razor`, `.css`, `.scss`, `.vue`, `.astro`, or `.cs`)
//! are targeted in `harness_verify`, this module validates syntax, directives,
//! tag/brace balance, and embedded blocks in milliseconds without running a full
//! solution compilation (`dotnet build` / `msbuild`) or executing project code.
//! Every result explicitly reports `validated` checks and `pending` checks.
use super::{Project, validate_verify_options};
use crate::adapters::parsers;
use harness_domain::{
    Failure,
    contracts::{VerifyAction, VerifyArgs},
    execution::OperationContext,
    process::{ProcessOutcome, Program, StopReason},
};
use serde_json::{Value, json};
use std::path::Path;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PerFileFormat {
    JavaScript,
    TypeScript,
    Python,
    Razor,
    Css,
    Scss,
    Vue,
    Astro,
    CSharp,
}

impl PerFileFormat {
    pub(super) fn from_path(path: &str) -> Option<Self> {
        let ext = Path::new(path).extension()?.to_str()?;
        if ext.eq_ignore_ascii_case("js")
            || ext.eq_ignore_ascii_case("mjs")
            || ext.eq_ignore_ascii_case("cjs")
            || ext.eq_ignore_ascii_case("jsx")
        {
            Some(Self::JavaScript)
        } else if ext.eq_ignore_ascii_case("ts")
            || ext.eq_ignore_ascii_case("tsx")
            || ext.eq_ignore_ascii_case("mts")
            || ext.eq_ignore_ascii_case("cts")
        {
            Some(Self::TypeScript)
        } else if ext.eq_ignore_ascii_case("py") {
            Some(Self::Python)
        } else if ext.eq_ignore_ascii_case("cshtml") || ext.eq_ignore_ascii_case("razor") {
            Some(Self::Razor)
        } else if ext.eq_ignore_ascii_case("css") {
            Some(Self::Css)
        } else if ext.eq_ignore_ascii_case("scss") {
            Some(Self::Scss)
        } else if ext.eq_ignore_ascii_case("vue") {
            Some(Self::Vue)
        } else if ext.eq_ignore_ascii_case("astro") {
            Some(Self::Astro)
        } else if ext.eq_ignore_ascii_case("cs") {
            Some(Self::CSharp)
        } else {
            None
        }
    }

    pub(super) const fn protocol_language(self) -> &'static str {
        match self {
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
            Self::Python => "python",
            Self::Razor => "razor",
            Self::Css => "css",
            Self::Scss => "scss",
            Self::Vue => "vue",
            Self::Astro => "astro",
            Self::CSharp => "csharp",
        }
    }

    pub(super) const fn mode_name(self) -> &'static str {
        match self {
            Self::JavaScript => "javascript_fast_per_file",
            Self::TypeScript => "typescript_syntax_only",
            Self::Python => "python_fast_per_file",
            Self::Razor => "razor_fast_per_file",
            Self::Css => "css_fast_per_file",
            Self::Scss => "scss_fast_per_file",
            Self::Vue => "vue_fast_per_file",
            Self::Astro => "astro_fast_per_file",
            Self::CSharp => "csharp_fast_per_file",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct PerFileTarget {
    pub(super) path: String,
    pub(super) format: PerFileFormat,
}

#[derive(Debug, Clone)]
pub(super) struct PerFileReport {
    #[allow(dead_code)]
    pub(super) passed: bool,
    pub(super) language: String,
    pub(super) verification_mode: String,
    pub(super) files_checked: Vec<String>,
    pub(super) validated: Vec<String>,
    pub(super) pending: Vec<String>,
    pub(super) diagnostics: Vec<Value>,
    pub(super) summary: String,
    pub(super) outcome: ProcessOutcome,
}

fn push_unique(list: &mut Vec<String>, item: impl Into<String>) {
    let value = item.into();
    if !list.iter().any(|existing| existing == &value) {
        list.push(value);
    }
}

fn supported_per_file_language(language: &str) -> bool {
    [
        "javascript",
        "js",
        "typescript",
        "ts",
        "python",
        "py",
        "razor",
        "cshtml",
        "dotnet",
        "csharp",
        "cs",
        "css",
        "scss",
        "vue",
        "astro",
    ]
    .iter()
    .any(|candidate| language.eq_ignore_ascii_case(candidate))
}

fn split_scope_entries(scope: &str) -> Vec<&str> {
    if scope.contains([',', ';', '\n']) {
        scope
            .split([',', ';', '\n'])
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect()
    } else {
        let trimmed = scope.trim();
        if trimmed.is_empty() {
            Vec::new()
        } else {
            vec![trimmed]
        }
    }
}

fn validate_relative_scope_path(project: &Project, entry: &str) -> Result<String, Failure> {
    if entry.is_empty() || entry == "." {
        return Err(("VERIFY_SCOPE", "Per-file scope requires an explicit file path"));
    }
    if entry.len() > 4096 {
        return Err(("PATH", "Scope too long"));
    }
    if entry.starts_with('-') {
        return Err(("VERIFY_SCOPE", "Per-file scope cannot start with '-'"));
    }
    if entry.contains(['*', '?', '[', ']', '{', '}', '!']) {
        return Err((
            "VERIFY_SCOPE",
            "Per-file scope cannot contain glob metacharacters",
        ));
    }
    let target = project.target(entry)?;
    if !target.is_file() {
        return Err((
            "VERIFY_SCOPE",
            "Per-file verification scope must resolve to an existing file",
        ));
    }
    let relative = target
        .strip_prefix(&project.root)
        .ok()
        .and_then(|path| path.to_str())
        .ok_or(("PATH", "Per-file scope must be relative UTF-8"))?;
    Ok(relative.replace('\\', "/"))
}

/// Determines whether `args` selects fast in-process per-file verification.
///
/// Compatibility rules:
/// - `.ts` / `.tsx` / `.mts` / `.cts` with `Build` or `None` always use fast in-process verification.
/// - `.cshtml` / `.razor`, `.css`, `.scss`, `.vue`, `.astro`, `.cs`, and multi-file scopes use fast
///   in-process verification when `action` is `Build` or `None`.
/// - `.js` / `.mjs` / `.cjs` / `.jsx` use fast in-process verification when `action` is `Build` or `None`
///   (while single-file `.js`/`.mjs`/`.cjs` with `action == Test` continues to use Node's test runner).
pub(super) fn resolve_per_file_targets(
    project: &Project,
    args: &VerifyArgs,
) -> Result<Option<Vec<PerFileTarget>>, Failure> {
    validate_verify_options(args)?;
    if args.package.is_some() && !args.language.eq_ignore_ascii_case("rust") {
        return Err(("VERIFY_PACKAGE", "package is supported only for Rust"));
    }
    if !supported_per_file_language(&args.language) {
        return Ok(None);
    }
    let is_dedicated_per_file_lang = ["razor", "cshtml", "css", "scss", "vue", "astro"]
        .iter()
        .any(|lang| args.language.eq_ignore_ascii_case(lang));
    let fast_engine = matches!(args.engine.as_deref(), Some("fast" | "syntax"));

    let Some(raw_scope) = args.scope.as_deref() else {
        if is_dedicated_per_file_lang || fast_engine {
            return Err((
                "VERIFY_SCOPE",
                "Fast per-file verification requires an explicit file path in scope",
            ));
        }
        return Ok(None);
    };
    if raw_scope.len() > 4096 {
        return Err(("PATH", "Scope too long"));
    }

    let entries = split_scope_entries(raw_scope);
    if entries.is_empty() {
        if is_dedicated_per_file_lang || fast_engine {
            return Err((
                "VERIFY_SCOPE",
                "Fast per-file verification requires an explicit file path in scope",
            ));
        }
        return Ok(None);
    }

    let is_multi = entries.len() > 1;
    let mut parsed_formats = Vec::with_capacity(entries.len());
    for entry in &entries {
        if let Some(format) = PerFileFormat::from_path(entry) {
            parsed_formats.push((*entry, format));
        } else {
            // If any entry is not a per-file source format (e.g. .csproj, .sln, directory),
            // only error if the caller explicitly asked for a dedicated per-file language or engine=fast.
            if is_dedicated_per_file_lang || fast_engine || is_multi {
                return Err((
                    "VERIFY_SCOPE",
                    "Fast per-file verification supports .js, .mjs, .cjs, .jsx, .ts, .tsx, .mts, .cts, .py, .cshtml, .razor, .css, .scss, .vue, .astro and .cs files",
                ));
            }
            return Ok(None);
        }
    }

    // Single Python file without engine=fast/syntax continues to use the stdlib py_compile / unittest runner in `python_plan`.
    if !is_multi && parsed_formats[0].1 == PerFileFormat::Python && !fast_engine {
        return Ok(None);
    }

    // Single JavaScript file with action == Test must use the Node test runner in `javascript_plan`.
    if !is_multi
        && parsed_formats[0].1 == PerFileFormat::JavaScript
        && matches!(args.action, Some(VerifyAction::Test))
        && !fast_engine
        && !args.language.eq_ignore_ascii_case("dotnet")
        && !args.language.eq_ignore_ascii_case("csharp")
        && !args.language.eq_ignore_ascii_case("razor")
        && !args.language.eq_ignore_ascii_case("cshtml")
    {
        return Ok(None);
    }

    // Validate all paths and check action compatibility.
    let mut targets = Vec::with_capacity(parsed_formats.len());
    for (entry, format) in parsed_formats {
        let normalized_path = match validate_relative_scope_path(project, entry) {
            Ok(path) => path,
            Err(err) => {
                if format == PerFileFormat::TypeScript {
                    return Err((
                        "VERIFY_SCOPE",
                        "TypeScript scope must be an explicit .ts or .tsx file",
                    ));
                }
                return Err(err);
            }
        };
        if matches!(args.action, Some(VerifyAction::Test)) {
            if format == PerFileFormat::TypeScript {
                return Err((
                    "UNSUPPORTED",
                    "TypeScript/TSX test execution is not supported; build performs native syntax validation only",
                ));
            }
            return Err((
                "UNSUPPORTED",
                "Fast per-file verification validates syntax and structure only; test execution is not supported for this file scope",
            ));
        }
        targets.push(PerFileTarget {
            path: normalized_path,
            format,
        });
    }

    Ok(Some(targets))
}

/// Backwards-compatible helper used by existing TypeScript tests and `verify.rs`.
pub(super) fn typescript_scope(
    project: &Project,
    args: &VerifyArgs,
) -> Result<Option<String>, Failure> {
    let Some(targets) = resolve_per_file_targets(project, args)? else {
        return Ok(None);
    };
    let joined = targets
        .iter()
        .map(|t| t.path.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Ok(Some(joined))
}

/// Replace Razor `@` transitions inside an inline `<script>` body with valid JS placeholders
/// so that Oxc can validate the surrounding JavaScript syntax without false-positive errors
/// on `@Model.Prop`, `@Html.Raw(...)`, `@Url.Action(...)`, `@(...)`, or `@Json.Serialize(...)`.
fn neutralize_razor_transitions_in_script(script: &str) -> (String, usize) {
    let bytes = script.as_bytes();
    let mut out = String::with_capacity(script.len());
    let mut transitions = 0usize;
    let mut i = 0;

    while i < bytes.len() {
        // Respect Razor comments @* ... *@ inside <script>
        if bytes[i..].starts_with(b"@*") {
            transitions += 1;
            i += 2;
            while i + 1 < bytes.len() && &bytes[i..i + 2] != b"*@" {
                if bytes[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        // Escaped @@ becomes literal @
        if bytes[i..].starts_with(b"@@") {
            out.push('@');
            i += 2;
            continue;
        }
        // Skip JS comments so @ inside comments is untouched
        if bytes[i..].starts_with(b"//") {
            let start = i;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            out.push_str(&script[start..i]);
            continue;
        }
        if bytes[i..].starts_with(b"/*") {
            let start = i;
            i += 2;
            while i + 1 < bytes.len() && &bytes[i..i + 2] != b"*/" {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            out.push_str(&script[start..i]);
            continue;
        }
        // Skip JS string literals ('...', "...", `...`) so @ inside strings is untouched
        if matches!(bytes[i], b'\'' | b'"' | b'`') {
            let quote = bytes[i];
            out.push(quote as char);
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    let ch = script[i..].chars().next().unwrap();
                    out.push(ch);
                    i += ch.len_utf8();
                    if i < bytes.len() {
                        let next_ch = script[i..].chars().next().unwrap();
                        out.push(next_ch);
                        i += next_ch.len_utf8();
                    }
                    continue;
                }
                if bytes[i] == quote {
                    out.push(quote as char);
                    i += 1;
                    break;
                }
                let ch = script[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
            }
            continue;
        }
        if bytes[i] == b'@' {
            let next = bytes.get(i + 1).copied();
            if next == Some(b'(') {
                if let Some(end) = parsers::razor::syntax::delimited_end(bytes, i + 1, b'(', b')') {
                    transitions += 1;
                    out.push_str("__razor_expr__");
                    for b in &bytes[i..end] {
                        if *b == b'\n' {
                            out.push('\n');
                        }
                    }
                    i = end;
                    continue;
                }
            } else if next == Some(b'{') {
                if let Some(end) = parsers::razor::syntax::block_end(bytes, i + 1) {
                    transitions += 1;
                    for b in &bytes[i..end] {
                        if *b == b'\n' {
                            out.push('\n');
                        }
                    }
                    i = end;
                    continue;
                }
            } else if next.is_some_and(|b| b.is_ascii_alphabetic() || b == b'_') {
                transitions += 1;
                let mut cursor = i + 1;
                while cursor < bytes.len() {
                    while cursor < bytes.len()
                        && (bytes[cursor].is_ascii_alphanumeric()
                            || matches!(bytes[cursor], b'_' | b'.'))
                    {
                        cursor += 1;
                    }
                    if matches!(bytes.get(cursor), Some(b'(' | b'[')) {
                        let left = bytes[cursor];
                        let right = if left == b'(' { b')' } else { b']' };
                        if let Some(end) =
                            parsers::razor::syntax::delimited_end(bytes, cursor, left, right)
                        {
                            cursor = end;
                            if bytes.get(cursor) == Some(&b'.') {
                                cursor += 1;
                                continue;
                            }
                        }
                    }
                    break;
                }
                out.push_str("__razor_expr__");
                for b in &bytes[i..cursor] {
                    if *b == b'\n' {
                        out.push('\n');
                    }
                }
                i = cursor;
                continue;
            }
        }

        let ch = script[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    (out, transitions)
}

/// Scan inline `<script>` blocks in a Razor `.cshtml` / `.razor` file and validate JS syntax.
fn check_razor_inline_scripts(
    path: &str,
    source: &str,
    context: &OperationContext,
    diagnostics: &mut Vec<Value>,
) -> Result<(usize, usize), Failure> {
    let mut cursor = 0usize;
    let mut checked = 0usize;
    let mut skipped_custom = 0usize;

    while cursor < source.len() {
        context.check()?;
        let Some(start) = source[cursor..].find('<').map(|n| cursor + n) else {
            break;
        };
        if source[start..].starts_with("<!--") {
            cursor = source[start + 4..]
                .find("-->")
                .map_or(source.len(), |n| start + 4 + n + 3);
            continue;
        }
        if source[start..].starts_with("@*") {
            cursor = source[start + 2..]
                .find("*@")
                .map_or(source.len(), |n| start + 2 + n + 2);
            continue;
        }
        let Some(tag) = parsers::vue::syntax::parse_tag(source, start) else {
            cursor = start + 1;
            continue;
        };
        cursor = tag.end;
        if tag.closing || !tag.name.eq_ignore_ascii_case("script") {
            continue;
        }
        if tag.self_closing {
            continue;
        }
        let Some((body_end, after)) = parsers::vue::syntax::closing_tag(source, cursor, "script")
        else {
            break;
        };
        let body = &source[cursor..body_end];
        let base_line = source.as_bytes()[..cursor]
            .iter()
            .filter(|b| **b == b'\n')
            .count()
            + 1;
        cursor = after;

        if parsers::vue::syntax::has_attribute(tag.attrs, "src") {
            continue;
        }
        let script_type = parsers::vue::syntax::attribute(tag.attrs, "type")
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let is_js = script_type.is_empty()
            || script_type == "module"
            || script_type == "text/javascript"
            || script_type == "application/javascript";
        if !is_js {
            skipped_custom += 1;
            continue;
        }

        let (sanitized, _transitions) = neutralize_razor_transitions_in_script(body);
        let virtual_path = if script_type == "module" {
            "inline.mjs"
        } else {
            "inline.js"
        };
        if let Err((code, message)) =
            parsers::ecmascript::syntax::validate_syntax(virtual_path, &sanitized, context)
        {
            let (rel_line, column) =
                parsers::ecmascript::syntax::first_error_location(virtual_path, &sanitized)
                    .unwrap_or((1, 1));
            let line = base_line + rel_line.saturating_sub(1);
            diagnostics.push(json!({
                "code": if code == "SYNTAX_ERROR" { "RAZOR_INLINE_JS_SYNTAX" } else { code },
                "message": format!("Inline <script> syntax error in {path} (line {line}, col {column}): {message}"),
                "path": path,
                "line": line,
                "column": column,
                "severity": "error"
            }));
        }
        checked += 1;
    }

    Ok((checked, skipped_custom))
}

/// Extract `@{ ... }` anonymous code blocks in Razor in addition to `@code` / `@functions` blocks.
fn collect_anonymous_razor_code_blocks(source: &str) -> Vec<(std::ops::Range<usize>, &'static str)> {
    let bytes = source.as_bytes();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"@*") {
            i += 2;
            while i + 1 < bytes.len() && &bytes[i..i + 2] != b"*@" {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        if bytes[i..].starts_with(b"@@") {
            i += 2;
            continue;
        }
        if bytes[i..].starts_with(b"@{") {
            let open = i + 1;
            if let Some(end) = parsers::razor::syntax::block_end(bytes, open) {
                blocks.push((open + 1..end - 1, "code_block"));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    blocks
}

fn verify_single_file(
    project: &Project,
    target: &PerFileTarget,
    context: &OperationContext,
    validated: &mut Vec<String>,
    pending: &mut Vec<String>,
    diagnostics: &mut Vec<Value>,
) -> Result<(), Failure> {
    context.check()?;
    let path = &target.path;
    let max_bytes = match target.format {
        PerFileFormat::JavaScript | PerFileFormat::TypeScript | PerFileFormat::Python => 512 * 1024,
        PerFileFormat::Css => 256 * 1024,
        PerFileFormat::Razor | PerFileFormat::Vue | PerFileFormat::Astro | PerFileFormat::Scss => {
            2 * 1024 * 1024
        }
        PerFileFormat::CSharp => 4 * 1024 * 1024,
    };
    let bytes = project.bytes_with_limit(path, context, max_bytes)?;
    let source = std::str::from_utf8(&bytes).map_err(|_| {
        (
            "ENCODING",
            match target.format {
                PerFileFormat::TypeScript => "TypeScript source must be UTF-8",
                _ => "Source file must be UTF-8",
            },
        )
    })?;

    match target.format {
        PerFileFormat::JavaScript | PerFileFormat::TypeScript => {
            let prefix_tag = if target.format == PerFileFormat::JavaScript {
                "js"
            } else {
                "ts"
            };
            match parsers::ecmascript::syntax::validate_syntax(path, source, context) {
                Ok(()) => {
                    push_unique(validated, format!("{prefix_tag}-syntax ({path})"));
                    if let Ok(symbols) =
                        parsers::ecmascript::symbols::collect(path, source, "", context)
                    {
                        push_unique(
                            validated,
                            format!(
                                "{prefix_tag}-ast-structure ({path}: {} symbols)",
                                symbols.len()
                            ),
                        );
                    }
                }
                Err(("SYNTAX_ERROR", message)) => {
                    let (line, column) =
                        parsers::ecmascript::syntax::first_error_location(path, source)
                            .unwrap_or((1, 1));
                    push_unique(validated, format!("{prefix_tag}-syntax ({path}: failed)"));
                    diagnostics.push(json!({
                        "code": "SYNTAX_ERROR",
                        "message": format!("{message} ({path}:{line}:{column})"),
                        "path": path,
                        "line": line,
                        "column": column,
                        "severity": "error"
                    }));
                }
                Err(err) => return Err(err),
            }
            if target.format == PerFileFormat::TypeScript {
                push_unique(
                    pending,
                    "typescript-project-typecheck (skipped for fast per-file check)",
                );
            } else {
                push_unique(
                    pending,
                    "module-Import-and-bundler-resolution (skipped for fast per-file check)",
                );
            }
            push_unique(pending, "browser-runtime-verification");
        }
        PerFileFormat::Razor => {
            // 1. Razor directives, expressions, comments, unescaped @media, and HTML tag balance
            let razor_diag = parsers::razor::syntax::diagnose(source, context)?;
            let error_count = razor_diag["errors"].as_u64().unwrap_or(0);
            if error_count == 0 {
                push_unique(validated, format!("razor-directives ({path})"));
                push_unique(validated, format!("razor-syntax-balance ({path})"));
                push_unique(validated, format!("razor-html-tag-balance ({path})"));
            } else {
                push_unique(
                    validated,
                    format!("razor-syntax-balance ({path}: {error_count} error(s))"),
                );
                if let Some(items) = razor_diag["diagnostics"].as_array() {
                    for item in items {
                        let mut diag = item.clone();
                        diag["path"] = json!(path);
                        diagnostics.push(diag);
                    }
                }
            }

            // 2. Embedded C# blocks (@code, @functions, @{ ... }) lexical & brace balance
            let mut csharp_blocks = Vec::new();
            let _ = parsers::csharp::syntax::collect_cshtml_blocks(
                source,
                "",
                context,
                &mut csharp_blocks,
                false,
            );
            let anon_blocks = collect_anonymous_razor_code_blocks(source);
            let total_blocks = csharp_blocks.len() + anon_blocks.len();
            let mut block_errors = 0usize;
            for (range, kind) in csharp_blocks.into_iter().chain(anon_blocks.into_iter()) {
                if let Some(block_src) = source.get(range.clone()) {
                    // Only validate pure C# brace/string/comment balance if the block does not embed raw HTML tags
                    if !block_src.contains("</")
                        && let Err((code, message)) = parsers::csharp::syntax::validate_syntax(
                            "razor_block.cs",
                            block_src,
                            context,
                        )
                    {
                        block_errors += 1;
                        let line = source.as_bytes()[..range.start]
                            .iter()
                            .filter(|b| **b == b'\n')
                            .count()
                            + 1;
                        diagnostics.push(json!({
                            "code": code,
                            "message": format!("Razor @{kind} C# block syntax error in {path} (near line {line}): {message}"),
                            "path": path,
                            "line": line,
                            "severity": "error"
                        }));
                    }
                }
            }
            if block_errors == 0 {
                push_unique(
                    validated,
                    format!("razor-csharp-blocks ({path}: {total_blocks} block(s))"),
                );
            } else {
                push_unique(
                    validated,
                    format!("razor-csharp-blocks ({path}: {block_errors} error(s))"),
                );
            }

            // 3. Embedded inline <script> blocks
            let (scripts_checked, _skipped) =
                check_razor_inline_scripts(path, source, context, diagnostics)?;
            if scripts_checked > 0 {
                push_unique(
                    validated,
                    format!("razor-embedded-scripts ({path}: {scripts_checked} block(s))"),
                );
            }

            // 4. Embedded inline <style> blocks
            if let Ok(styles) = parsers::css::symbols::collect(source, "", true, context)
                && !styles.is_empty()
            {
                push_unique(
                    validated,
                    format!("razor-embedded-styles ({path}: {} rule(s))", styles.len()),
                );
            }

            push_unique(
                pending,
                "dotnet-msbuild-compile (skipped for fast per-file check)",
            );
            push_unique(
                pending,
                "csharp-model-and-helper-typecheck (skipped for fast per-file check)",
            );
            push_unique(pending, "browser-runtime-verification");
        }
        PerFileFormat::Css => {
            match parsers::scss::syntax::validate(source, context) {
                Ok(_) => {
                    let rules = parsers::css::symbols::collect(source, "", false, context)
                        .map(|s| s.len())
                        .unwrap_or(0);
                    push_unique(validated, format!("css-syntax ({path}: {rules} rule(s))"));
                }
                Err((code, message)) => {
                    let (line, column) = parsers::scss::syntax::first_error(source)
                        .map(|d| (d.line, d.column))
                        .unwrap_or((1, 1));
                    push_unique(validated, format!("css-syntax ({path}: failed)"));
                    diagnostics.push(json!({
                        "code": code,
                        "message": format!("{message} ({path}:{line}:{column})"),
                        "path": path,
                        "line": line,
                        "column": column,
                        "severity": "error"
                    }));
                }
            }
            push_unique(pending, "browser-runtime-verification");
        }
        PerFileFormat::Scss => {
            match parsers::scss::syntax::validate(source, context) {
                Ok(_) => {
                    push_unique(validated, format!("scss-syntax ({path})"));
                }
                Err((code, message)) => {
                    let (line, column) = parsers::scss::syntax::first_error(source)
                        .map(|d| (d.line, d.column))
                        .unwrap_or((1, 1));
                    push_unique(validated, format!("scss-syntax ({path}: failed)"));
                    diagnostics.push(json!({
                        "code": code,
                        "message": format!("{message} ({path}:{line}:{column})"),
                        "path": path,
                        "line": line,
                        "column": column,
                        "severity": "error"
                    }));
                }
            }
            push_unique(
                pending,
                "scss-module-imports-resolution (skipped for fast per-file check)",
            );
            push_unique(pending, "browser-runtime-verification");
        }
        PerFileFormat::Vue => {
            match parsers::vue::syntax::validate(path, source, context) {
                Ok(info) => {
                    let scripts = info["scriptsChecked"].as_u64().unwrap_or(0);
                    push_unique(
                        validated,
                        format!("vue-sfc-syntax ({path}: {scripts} script(s))"),
                    );
                }
                Err((code, message)) => {
                    push_unique(validated, format!("vue-sfc-syntax ({path}: failed)"));
                    diagnostics.push(json!({
                        "code": code,
                        "message": format!("{message} ({path})"),
                        "path": path,
                        "severity": "error"
                    }));
                }
            }
            push_unique(
                pending,
                "vue-template-and-project-typecheck (skipped for fast per-file check)",
            );
            push_unique(pending, "browser-runtime-verification");
        }
        PerFileFormat::Astro => {
            match parsers::astro::syntax::validate(path, source, context) {
                Ok(info) => {
                    let scripts = info["scriptsChecked"].as_u64().unwrap_or(0);
                    push_unique(
                        validated,
                        format!("astro-component-syntax ({path}: {scripts} script(s))"),
                    );
                }
                Err((code, message)) => {
                    push_unique(validated, format!("astro-component-syntax ({path}: failed)"));
                    diagnostics.push(json!({
                        "code": code,
                        "message": format!("{message} ({path})"),
                        "path": path,
                        "severity": "error"
                    }));
                }
            }
            push_unique(
                pending,
                "astro-project-typecheck (skipped for fast per-file check)",
            );
            push_unique(pending, "browser-runtime-verification");
        }
        PerFileFormat::CSharp => {
            match parsers::csharp::syntax::validate_syntax(path, source, context) {
                Ok(_) => {
                    push_unique(validated, format!("csharp-syntax-balance ({path})"));
                }
                Err((code, message)) => {
                    push_unique(validated, format!("csharp-syntax-balance ({path}: failed)"));
                    diagnostics.push(json!({
                        "code": code,
                        "message": format!("{message} ({path})"),
                        "path": path,
                        "severity": "error"
                    }));
                }
            }
            push_unique(
                pending,
                "dotnet-msbuild-compile (skipped for fast per-file check)",
            );
            push_unique(
                pending,
                "csharp-semantic-typecheck (skipped for fast per-file check)",
            );
        }
        PerFileFormat::Python => {
            match parsers::python::syntax::validate_syntax(source, context) {
                Ok(()) => {
                    push_unique(validated, format!("python-syntax ({path})"));
                    if let Ok(symbols) = parsers::python::symbols::collect(source, "", context) {
                        push_unique(
                            validated,
                            format!("python-ast-structure ({path}: {} symbols)", symbols.len()),
                        );
                    }
                }
                Err(("SYNTAX_ERROR", message)) => {
                    let (line, column) =
                        parsers::python::syntax::first_error_location(source).unwrap_or((1, 1));
                    push_unique(validated, format!("python-syntax ({path}: failed)"));
                    diagnostics.push(json!({
                        "code": "SYNTAX_ERROR",
                        "message": format!("{message} ({path}:{line}:{column})"),
                        "path": path,
                        "line": line,
                        "column": column,
                        "severity": "error"
                    }));
                }
                Err(err) => return Err(err),
            }
            push_unique(
                pending,
                "python-unit-tests (skipped for fast per-file check)",
            );
            push_unique(pending, "runtime-integration-verification");
        }
    }

    Ok(())
}

/// Runs fast in-process verification and returns a full structured report.
pub(super) fn run_per_file_verification(
    project: &Project,
    args: &VerifyArgs,
    context: &OperationContext,
) -> Result<PerFileReport, Failure> {
    context.check()?;
    let started = Instant::now();
    let targets = resolve_per_file_targets(project, args)?.ok_or((
        "VERIFY_SCOPE",
        "Expected an explicit file scope for fast per-file verification",
    ))?;

    let mut validated = Vec::new();
    let mut pending = Vec::new();
    let mut diagnostics = Vec::new();
    let mut files_checked = Vec::with_capacity(targets.len());

    // If a .js/.css file is verified inside a .NET workspace (or with language="dotnet"/"csharp"),
    // also note that dotnet-msbuild-compile was skipped.
    let is_dotnet_context = args.language.eq_ignore_ascii_case("dotnet")
        || args.language.eq_ignore_ascii_case("csharp")
        || targets.iter().any(|t| {
            matches!(
                t.format,
                PerFileFormat::Razor | PerFileFormat::CSharp
            )
        });

    for target in &targets {
        files_checked.push(target.path.clone());
        verify_single_file(
            project,
            target,
            context,
            &mut validated,
            &mut pending,
            &mut diagnostics,
        )?;
    }

    if is_dotnet_context {
        push_unique(
            &mut pending,
            "dotnet-msbuild-compile (skipped for fast per-file check)",
        );
    }

    let passed = diagnostics.is_empty();
    let all_ts = targets
        .iter()
        .all(|t| t.format == PerFileFormat::TypeScript);
    let first_format = targets[0].format;
    let same_format = targets.iter().all(|t| t.format == first_format);

    // Preserve exact backwards compatibility for single/multi TypeScript files:
    // `language: "typescript"` and `verificationMode: "typescript_syntax_only"`.
    let (language, verification_mode) = if all_ts {
        ("typescript".to_owned(), "typescript_syntax_only".to_owned())
    } else if same_format {
        (
            first_format.protocol_language().to_owned(),
            first_format.mode_name().to_owned(),
        )
    } else {
        ("multi_file".to_owned(), "fast_per_file_syntax".to_owned())
    };

    let status_label = if passed { "PASSED" } else { "FAILED" };
    let summary = format!(
        "Fast per-file verification {status_label} ({} file(s): {}). Validated: [{}]. Pending (not run): [{}].",
        files_checked.len(),
        files_checked.join(", "),
        validated.join(", "),
        pending.join(", ")
    );

    let (exit_code, stdout, stderr) = if passed {
        (Some(0), summary.as_bytes().to_vec(), Vec::new())
    } else {
        let error_lines: Vec<String> = diagnostics
            .iter()
            .map(|d| {
                let code = d["code"].as_str().unwrap_or("SYNTAX_ERROR");
                let msg = d["message"]
                    .as_str()
                    .unwrap_or("Syntax or structural validation failed");
                format!("[{code}] {msg}")
            })
            .collect();
        let stderr_text = format!("{}\n{}", error_lines.join("\n"), summary);
        (Some(1), Vec::new(), stderr_text.into_bytes())
    };

    context.check()?;
    Ok(PerFileReport {
        passed,
        language,
        verification_mode,
        files_checked,
        validated,
        pending,
        diagnostics,
        summary,
        outcome: ProcessOutcome {
            exit_code,
            stdout,
            stderr,
            elapsed: started.elapsed(),
            reason: StopReason::Exited,
            tree_termination_confirmed: true,
        },
    })
}

/// Backwards-compatible entry point used by `task_execution.rs` for background jobs
/// and direct unit tests in `tests/javascript.rs`.
pub(in crate::adapters) fn run_typescript_syntax(
    root: &Path,
    args: &VerifyArgs,
    context: &OperationContext,
) -> Result<ProcessOutcome, Failure> {
    let project = Project::new(root).map_err(|_| ("PATH", "Workspace unavailable"))?;
    let report = run_per_file_verification(&project, args, context)?;
    Ok(report.outcome)
}

/// Builds `validated`, `pending`, and `summary` fields for process-backed verifiers
/// so every `harness_verify` call consistently reports what was checked and what remains pending.
pub(super) fn process_verification_transparency(
    program: Option<Program>,
    angular: bool,
    args: &VerifyArgs,
    passed: bool,
) -> (Vec<String>, Vec<String>, String) {
    let scope = args.scope.as_deref().unwrap_or(".");
    let is_test = matches!(args.action, Some(VerifyAction::Test));
    let status = if passed { "passed" } else { "failed" };

    let (validated, pending) = if angular {
        (
            vec![
                format!("angular-typescript-compile ({scope}: {status})"),
                format!("angular-template-typecheck ({scope}: {status})"),
            ],
            vec![
                "angular-bundle-emit (noEmit=true)".to_owned(),
                "browser-runtime-verification".to_owned(),
            ],
        )
    } else {
        match program {
            Some(Program::Dotnet | Program::Msbuild) => {
                if is_test {
                    (
                        vec![format!("dotnet-test ({scope}: {status})")],
                        vec!["browser-runtime-verification".to_owned()],
                    )
                } else {
                    (
                        vec![format!("dotnet-compile ({scope}: {status})")],
                        vec![
                            "dotnet-Automated-tests (action=build)".to_owned(),
                            "razor-runtime-view-precompilation (if not enabled in csproj)"
                                .to_owned(),
                            "browser-runtime-verification".to_owned(),
                        ],
                    )
                }
            }
            Some(Program::Node) => {
                if is_test {
                    (
                        vec![format!("node-test-runner ({scope}: {status})")],
                        vec!["browser-runtime-verification".to_owned()],
                    )
                } else {
                    (
                        vec![format!("node-syntax-check ({scope}: {status})")],
                        vec![
                            "javascript-unit-tests (action=build)".to_owned(),
                            "browser-runtime-verification".to_owned(),
                        ],
                    )
                }
            }
            Some(Program::Python) => {
                if is_test {
                    (
                        vec![format!("python-test-suite ({scope}: {status})")],
                        vec!["runtime-integration-verification".to_owned()],
                    )
                } else {
                    (
                        vec![format!("python-py-compile-syntax ({scope}: {status})")],
                        vec!["python-unit-tests (action=build)".to_owned()],
                    )
                }
            }
            Some(Program::Go) => {
                if is_test {
                    (
                        vec![format!("go-test ({scope}: {status})")],
                        vec!["runtime-integration-verification".to_owned()],
                    )
                } else {
                    (
                        vec![format!("go-build ({scope}: {status})")],
                        vec!["go-tests (action=build)".to_owned()],
                    )
                }
            }
            _ => {
                let step = match args.action {
                    Some(VerifyAction::Test) => "cargo-test",
                    Some(VerifyAction::Build) => "cargo-build",
                    None => "cargo-check",
                };
                let pending = if is_test {
                    vec!["end-to-end-runtime-verification".to_owned()]
                } else {
                    vec!["cargo-test (skipped for check/build)".to_owned()]
                };
                (vec![format!("{step} ({scope}: {status})")], pending)
            }
        }
    };

    let summary = format!(
        "Verification {}: validated [{}]; pending [{}].",
        if passed { "PASSED" } else { "FAILED" },
        validated.join(", "),
        pending.join(", ")
    );
    (validated, pending, summary)
}

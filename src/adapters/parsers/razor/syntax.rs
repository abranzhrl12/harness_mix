//! Conservative Razor checks, not an ASP.NET compiler or a C# type checker.
//! Explicit HTML nesting, section declarations and literal CSS transitions are checked.
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};
use std::collections::HashSet;

struct Finding {
    code: &'static str,
    message: &'static str,
    at: usize,
}

fn finding(out: &mut Vec<Finding>, code: &'static str, message: &'static str, at: usize) {
    // Keep output bounded even for pathological input.
    if out.len() < 100 {
        out.push(Finding { code, message, at });
    }
}

fn word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn skip_space(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Skip C# quoted literals while matching a block. Verbatim strings escape quotes twice.
fn quoted(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let verbatim = start > 0 && bytes[start - 1] == b'@';
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == quote {
            if verbatim && bytes.get(i + 1) == Some(&quote) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        if !verbatim && bytes[i] == b'\\' {
            i += 1;
        }
        i += 1;
    }
    bytes.len()
}

fn tag_name_at(bytes: &[u8], start: usize, name: &[u8]) -> bool {
    bytes.len() >= start + name.len()
        && bytes[start..start + name.len()].eq_ignore_ascii_case(name)
        && bytes
            .get(start + name.len())
            .is_none_or(|b| b.is_ascii_whitespace() || matches!(*b, b'>' | b'/'))
}

/// Advance past a `<script>...</script>` element. Braces inside JavaScript
/// (strings, template literals, regexes) are not Razor block delimiters.
fn raw_script_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len() && bytes[i] != b'>' {
        i += 1;
    }
    i = (i + 1).min(bytes.len());
    while i < bytes.len() {
        if bytes[i] == b'<'
            && bytes.get(i + 1) == Some(&b'/')
            && tag_name_at(bytes, i + 2, b"script")
        {
            let mut end = i + 2;
            while end < bytes.len() && bytes[end] != b'>' {
                end += 1;
            }
            return (end + 1).min(bytes.len());
        }
        i += 1;
    }
    bytes.len()
}

pub(in crate::adapters) fn delimited_end(
    bytes: &[u8],
    open: usize,
    left: u8,
    right: u8,
) -> Option<usize> {
    let mut depth = 1usize;
    let mut i = open + 1;
    while i < bytes.len() {
        if left == b'{' && bytes[i] == b'<' && tag_name_at(bytes, i + 1, b"script") {
            i = raw_script_end(bytes, i);
        } else if bytes[i..].starts_with(b"@*") || bytes[i..].starts_with(b"/*") {
            let close = if bytes[i] == b'@' { b"*@" } else { b"*/" };
            i += 2;
            while i + 1 < bytes.len() && &bytes[i..i + 2] != close {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
        } else if bytes[i..].starts_with(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if bytes[i..].starts_with(b"<!--") {
            i += 4;
            while i + 2 < bytes.len() && &bytes[i..i + 3] != b"-->" {
                i += 1;
            }
            i = (i + 3).min(bytes.len());
        } else if bytes[i] == b'"' || (bytes[i] == b'\'' && bytes.get(i + 2) == Some(&b'\'')) {
            i = quoted(bytes, i);
        } else {
            if bytes[i] == left {
                depth += 1;
            } else if bytes[i] == right {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            i += 1;
        }
    }
    None
}

pub(in crate::adapters) fn block_end(bytes: &[u8], open: usize) -> Option<usize> {
    delimited_end(bytes, open, b'{', b'}')
}

/// HTML quotes are not C# strings: backslashes do not escape the delimiter.
/// Razor expressions inside an attribute temporarily switch back to C# quoting.
fn attribute_end(
    bytes: &[u8],
    start: usize,
    quoted: bool,
    pattern: bool,
    out: &mut Vec<Finding>,
) -> usize {
    let quote = quoted.then(|| bytes[start]);
    let value_start = start + usize::from(quoted);
    let mut i = value_start;
    while i < bytes.len()
        && !quote.map_or_else(
            || bytes[i].is_ascii_whitespace() || bytes[i] == b'>' || bytes[i..].starts_with(b"/>"),
            |quote| bytes[i] == quote,
        )
    {
        if bytes[i] != b'@' {
            i += 1;
            continue;
        }
        let at = i;
        if bytes[i..].starts_with(b"@@") {
            i += 2;
            continue;
        }
        if bytes[i..].starts_with(b"@*") {
            i += 2;
            while i + 1 < bytes.len() && !bytes[i..].starts_with(b"*@") {
                i += 1;
            }
            if i + 1 >= bytes.len() {
                finding(
                    out,
                    "RAZOR_COMMENT",
                    "Unterminated Razor comment (@* ... *@)",
                    at,
                );
                return bytes.len();
            }
            i += 2;
            continue;
        }
        // Razor leaves ordinary email addresses literal. A regex character class
        // before @ is not an email local part and still starts a Razor transition.
        let email = i > value_start
            && (word(bytes[i - 1]) || matches!(bytes[i - 1], b'.' | b'-' | b'+'))
            && bytes.get(i + 1).is_some_and(|b| word(*b))
            && (!pattern
                || bytes[value_start..i]
                    .iter()
                    .all(|b| word(*b) || matches!(*b, b'.' | b'-' | b'+')));
        if email {
            i += 1;
            continue;
        }
        let next = bytes.get(i + 1).copied();
        let identifier = next.is_some_and(|b| b.is_ascii_alphabetic() || b == b'_' || b >= 128);
        if next != Some(b'(') && (!identifier || (pattern && i != value_start)) {
            finding(
                out,
                "RAZOR_ATTRIBUTE_TRANSITION",
                "Invalid or literal @ in HTML attribute; escape a literal @ as @@ (including regular expressions)",
                at,
            );
            i += 1;
            continue;
        }
        i += 1;
        if identifier {
            while i < bytes.len() && (word(bytes[i]) || bytes[i] >= 128 || bytes[i] == b'.') {
                i += 1;
            }
        }
        // Skip calls/indexers as well as explicit @(...) expressions so their
        // C# strings may contain the same quote as the surrounding HTML value.
        while matches!(bytes.get(i), Some(b'(' | b'[')) {
            let left = bytes[i];
            let right = if left == b'(' { b')' } else { b']' };
            if let Some(end) = delimited_end(bytes, i, left, right) {
                if skip_space(bytes, i + 1) == end - 1 && i == at + 1 {
                    finding(
                        out,
                        "RAZOR_ATTRIBUTE_EXPRESSION",
                        "Razor explicit attribute expression cannot be empty",
                        at,
                    );
                }
                i = end;
            } else {
                finding(
                    out,
                    "RAZOR_EXPRESSION_UNCLOSED",
                    "Razor attribute expression is missing a closing ')' or ']'",
                    at,
                );
                return bytes.len();
            }
            while i < bytes.len() && (word(bytes[i]) || bytes[i] >= 128 || bytes[i] == b'.') {
                i += 1;
            }
        }
    }
    if quoted && i < bytes.len() { i + 1 } else { i }
}

fn pattern_attribute(bytes: &[u8], tag_end: usize, equals: usize) -> bool {
    let name_end = bytes[tag_end..equals]
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(tag_end, |n| tag_end + n + 1);
    let name_start = bytes[tag_end..name_end]
        .iter()
        .rposition(|b| !word(*b) && *b != b'-')
        .map_or(tag_end, |n| tag_end + n + 1);
    bytes[name_start..name_end].eq_ignore_ascii_case(b"pattern")
}

fn scan(source: &str, context: &OperationContext) -> Result<Vec<Finding>, Failure> {
    if source.len() > 2 * 1024 * 1024 {
        return Err(("PARSER_LIMIT", "Basic Razor parser is limited to 2 MiB"));
    }
    context.check()?;
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut stack: Vec<(String, usize)> = Vec::new();
    let mut sections = HashSet::new();
    let mut section_count = 0;
    let mut i = 0;
    let mut raw: Option<String> = None;
    while i < bytes.len() {
        if i % 1024 == 0 {
            context.check()?;
        }
        // Razor comments disappear before template parsing; HTML comments do not.
        if bytes[i..].starts_with(b"@*") {
            let start = i;
            i += 2;
            while i + 1 < bytes.len() && &bytes[i..i + 2] != b"*@" {
                i += 1;
            }
            if i + 1 >= bytes.len() {
                finding(
                    &mut out,
                    "RAZOR_COMMENT",
                    "Unterminated Razor comment (@* ... *@)",
                    start,
                );
                break;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        if bytes[i..].starts_with(b"@@") {
            i += 2;
            continue;
        }
        // Line directives can contain generic C# type arguments, not HTML tags.
        let mut line_directive = false;
        for directive in [
            "@model",
            "@inherits",
            "@inject",
            "@namespace",
            "@addTagHelper",
            "@removeTagHelper",
            "@tagHelperPrefix",
        ] {
            if bytes[i..].starts_with(directive.as_bytes())
                && bytes
                    .get(i + directive.len())
                    .is_some_and(|b| b.is_ascii_whitespace())
            {
                line_directive = true;
                break;
            }
        }
        if line_directive {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Explicit expressions and control-flow headers can contain '<' comparisons.
        let mut paren_open = bytes[i..].starts_with(b"@(").then_some(i + 1);
        for directive in [
            "@if", "@for", "@foreach", "@while", "@switch", "@using", "@lock",
        ] {
            if bytes[i..].starts_with(directive.as_bytes())
                && !bytes.get(i + directive.len()).is_some_and(|b| word(*b))
            {
                let open = skip_space(bytes, i + directive.len());
                if bytes.get(open) == Some(&b'(') {
                    paren_open = Some(open);
                } else if directive == "@using" {
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                    line_directive = true;
                    break;
                }
            }
        }
        if line_directive {
            continue;
        }
        if let Some(open) = paren_open {
            if let Some(end) = delimited_end(bytes, open, b'(', b')') {
                i = end;
                continue;
            }
            finding(
                &mut out,
                "RAZOR_EXPRESSION_UNCLOSED",
                "Razor expression is missing its closing ')'",
                i,
            );
            break;
        }
        if bytes[i..].starts_with(b"@media") && !bytes.get(i + 6).is_some_and(|b| word(*b)) {
            finding(
                &mut out,
                "RAZOR_UNESCAPED_MEDIA",
                "Unescaped @media can cause CS0103: 'media' does not exist in the current context; use @@media in Razor CSS",
                i,
            );
            i += 6;
            continue;
        }
        if bytes[i..].starts_with(b"@section") && !bytes.get(i + 8).is_some_and(|b| word(*b)) {
            section_count += 1;
            if section_count > 100 {
                return Err((
                    "PARSER_LIMIT",
                    "Basic Razor parser supports at most 100 section declarations",
                ));
            }
            let start = i;
            let name_start = skip_space(bytes, i + 8);
            let mut name_end = name_start;
            while name_end < bytes.len() && word(bytes[name_end]) {
                name_end += 1;
            }
            let open = skip_space(bytes, name_end);
            if name_start == name_end
                || bytes[name_start].is_ascii_digit()
                || bytes.get(open) != Some(&b'{')
            {
                finding(
                    &mut out,
                    "RAZOR_SECTION",
                    "Expected @section Name { ... }",
                    start,
                );
                i += 8;
                continue;
            }
            if !sections.insert(source[name_start..name_end].to_string()) {
                finding(
                    &mut out,
                    "RAZOR_SECTION_DUPLICATE",
                    "Duplicate Razor section name",
                    start,
                );
            }
            if block_end(bytes, open).is_none() {
                finding(
                    &mut out,
                    "RAZOR_SECTION_UNCLOSED",
                    "Razor @section is missing its closing '}'",
                    start,
                );
            }
            // Continue through the markup body to check CSS and HTML too.
            i = open + 1;
            continue;
        }
        // Opaque C# blocks and explicit expressions may contain comparisons/generics.
        let mut code_open = None;
        if bytes[i..].starts_with(b"@{") {
            code_open = Some(i + 1);
        }
        for directive in [b"@functions".as_slice(), b"@code".as_slice()] {
            if bytes[i..].starts_with(directive)
                && !bytes.get(i + directive.len()).is_some_and(|b| word(*b))
            {
                let open = skip_space(bytes, i + directive.len());
                if bytes.get(open) == Some(&b'{') {
                    code_open = Some(open);
                }
            }
        }
        if let Some(open) = code_open {
            if let Some(end) = block_end(bytes, open) {
                i = end;
                continue;
            }
            finding(
                &mut out,
                "RAZOR_BLOCK_UNCLOSED",
                "Razor code block is missing its closing '}'",
                i,
            );
            break;
        }
        if bytes[i..].starts_with(b"<!--") {
            // Razor transitions inside HTML comments still execute.
            i += 4;
            while i + 2 < bytes.len() && &bytes[i..i + 3] != b"-->" {
                if bytes[i..].starts_with(b"@@") {
                    i += 2;
                    continue;
                }
                if bytes[i..].starts_with(b"@media") {
                    finding(
                        &mut out,
                        "RAZOR_UNESCAPED_MEDIA",
                        "Unescaped @media can cause CS0103; use @@media in Razor CSS",
                        i,
                    );
                }
                i += 1;
            }
            i = (i + 3).min(bytes.len());
            continue;
        }
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let start = i;
        let closing = bytes.get(i + 1) == Some(&b'/');
        let name_start = i + if closing { 2 } else { 1 };
        let mut end = name_start;
        while end < bytes.len() && (word(bytes[end]) || matches!(bytes[end], b'-' | b':')) {
            end += 1;
        }
        if end == name_start || !bytes[name_start].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let name = source[name_start..end].to_ascii_lowercase();
        if raw.as_ref().is_some_and(|tag| !closing || *tag != name) {
            i += 1;
            continue;
        }
        i = end;
        while i < bytes.len() && bytes[i] != b'>' {
            if bytes[i] == b'=' {
                let pattern = pattern_attribute(bytes, end, i);
                i = skip_space(bytes, i + 1);
                if i < bytes.len() && bytes[i] != b'>' {
                    i = attribute_end(
                        bytes,
                        i,
                        matches!(bytes[i], b'"' | b'\''),
                        pattern,
                        &mut out,
                    );
                }
            } else if matches!(bytes[i], b'"' | b'\'') {
                let equals = bytes[end..i]
                    .iter()
                    .rposition(|b| !b.is_ascii_whitespace())
                    .map(|p| p + end);
                let pattern = equals
                    .filter(|p| bytes[*p] == b'=')
                    .is_some_and(|p| pattern_attribute(bytes, end, p));
                i = attribute_end(bytes, i, true, pattern, &mut out);
            } else {
                i += 1;
            }
        }
        if i == bytes.len() {
            finding(
                &mut out,
                "RAZOR_HTML_TAG",
                "Unterminated HTML tag or attribute",
                start,
            );
            break;
        }
        let self_closing = i > start && bytes[i - 1] == b'/';
        i += 1;
        if closing {
            raw = None;
            if stack.last().is_some_and(|(tag, _)| *tag == name) {
                stack.pop();
            } else {
                finding(
                    &mut out,
                    "RAZOR_HTML_UNBALANCED",
                    "HTML closing tag does not match the open tag",
                    start,
                );
            }
        } else if !self_closing
            && !matches!(
                name.as_str(),
                "area"
                    | "base"
                    | "br"
                    | "col"
                    | "embed"
                    | "hr"
                    | "img"
                    | "input"
                    | "link"
                    | "meta"
                    | "param"
                    | "source"
                    | "track"
                    | "wbr"
            )
        {
            if matches!(name.as_str(), "script" | "style" | "textarea" | "title") {
                raw = Some(name.clone());
            }
            stack.push((name, start));
        }
    }
    for (_, at) in stack {
        finding(
            &mut out,
            "RAZOR_HTML_UNCLOSED",
            "HTML opening tag has no matching closing tag",
            at,
        );
    }
    Ok(out)
}

pub(in crate::adapters) fn diagnose(
    source: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    let findings = scan(source, context)?;
    let diagnostics: Vec<Value> = findings.iter().map(|f| {
        let line = source.as_bytes()[..f.at].iter().filter(|b| **b == b'\n').count() + 1;
        json!({"code":f.code,"message":f.message,"line":line,"byteOffset":f.at,"severity":"error"})
    }).collect();
    Ok(
        json!({"status":"checked","language":"razor","validation":"basic_razor","errors":findings.len(),"diagnostics":diagnostics,
        "limitations":["Basic lexical checks only; C# expressions and types require an ASP.NET build", "HTML checks require explicit balanced tags; optional end tags and conditional markup may require manual review"],
        "diagnosticLimit":100}),
    )
}

pub(in crate::adapters) fn validate(
    _path: &str,
    source: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    let findings = scan(source, context)?;
    if let Some(first) = findings.first() {
        return Err((first.code, first.message));
    }
    diagnose(source, context)
}

pub(in crate::adapters) fn validate_against_baseline(
    original: &str,
    updated: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    let updated_findings = scan(updated, context)?;
    if updated_findings.is_empty() {
        return diagnose(updated, context);
    }
    if let Some(strict) = updated_findings.iter().find(|f| {
        !matches!(
            f.code,
            "RAZOR_HTML_UNBALANCED" | "RAZOR_HTML_UNCLOSED"
        )
    }) {
        return Err((strict.code, strict.message));
    }
    let original_findings = scan(original, context)?;
    let orig_unbalanced = original_findings
        .iter()
        .filter(|f| matches!(f.code, "RAZOR_HTML_UNBALANCED" | "RAZOR_HTML_UNCLOSED"))
        .count();
    let upd_unbalanced = updated_findings.len();
    if orig_unbalanced > 0 && upd_unbalanced <= orig_unbalanced {
        let mut diag = diagnose(updated, context)?;
        diag["status"] = json!("valid_with_preexisting_warnings");
        diag["validation"] = json!("differential_razor");
        diag["preexistingHtmlBalanceWarnings"] = json!(upd_unbalanced);
        return Ok(diag);
    }
    let first = &updated_findings[0];
    Err((first.code, first.message))
}

#[cfg(test)]
#[path = "syntax_tests.rs"]
mod tests;

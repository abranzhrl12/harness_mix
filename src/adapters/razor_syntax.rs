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

pub(super) fn delimited_end(bytes: &[u8], open: usize, left: u8, right: u8) -> Option<usize> {
    let mut depth = 1usize;
    let mut i = open + 1;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"@*") || bytes[i..].starts_with(b"/*") {
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

pub(super) fn block_end(bytes: &[u8], open: usize) -> Option<usize> {
    delimited_end(bytes, open, b'{', b'}')
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
            if matches!(bytes[i], b'"' | b'\'') {
                i = quoted(bytes, i);
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

pub(super) fn diagnose(source: &str, context: &OperationContext) -> Result<Value, Failure> {
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

pub(super) fn validate(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_realistic_section_and_reports_basic_scope() {
        let source = r#"@model IEnumerable<Example>
@{ var value = "<not-a-tag>"; }
@section Scripts { <script>if (1 < 2) { alert("}"); }</script> }
<style>@@media screen { .x { color: red; } }</style><div><input disabled></div>
@if (a<b) { <p>@(a<b ? "yes" : "no")</p> }"#;
        let result = validate("x.cshtml", source, &OperationContext::default()).unwrap();
        assert_eq!(result["validation"], "basic_razor");
        assert_eq!(result["errors"], 0);
    }

    #[test]
    fn catches_media_with_line_and_compiler_hint_but_skips_razor_comments() {
        let result = diagnose(
            "@* @media ignored *@\n<style>@media screen {}</style>",
            &OperationContext::default(),
        )
        .unwrap();
        assert_eq!(result["errors"], 1);
        assert_eq!(result["diagnostics"][0]["line"], 2);
        assert!(
            result["diagnostics"][0]["message"]
                .as_str()
                .unwrap()
                .contains("CS0103")
        );
    }

    #[test]
    fn catches_sections_and_unbalanced_html() {
        for (source, code) in [
            ("@section Scripts <div></div>", "RAZOR_SECTION"),
            ("@section Scripts { <div></div>", "RAZOR_SECTION_UNCLOSED"),
            ("@section A {} @section A {}", "RAZOR_SECTION_DUPLICATE"),
            ("<div><span></div>", "RAZOR_HTML_UNBALANCED"),
            ("<main>", "RAZOR_HTML_UNCLOSED"),
        ] {
            let result = diagnose(source, &OperationContext::default()).unwrap();
            assert!(
                result["diagnostics"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d["code"] == code),
                "{source}: {result}"
            );
            assert!(validate("x.cshtml", source, &OperationContext::default()).is_err());
        }
    }

    #[test]
    fn escaped_transitions_comments_unicode_and_raw_text_do_not_create_tags() {
        let source = "@* <bad> *@<!-- <bad> --><div title='a > b'>á 😀</div><script>const s = '<fake>'; if (a < b) {}</script><style>@@media screen {}</style>";
        assert!(validate("x.cshtml", source, &OperationContext::default()).is_ok());
    }

    #[test]
    fn editing_and_csharp_entry_points_use_razor_checks() {
        let ctx = OperationContext::default();
        let valid = "<style>@@media screen {}</style>";
        assert_eq!(
            super::super::edit::syntax("x.CSHTML", valid, &ctx).unwrap()["validation"],
            "basic_razor"
        );
        assert_eq!(
            super::super::csharp_syntax::validate_syntax("x.CSHTML", valid, &ctx).unwrap()["language"],
            "razor"
        );
        assert_eq!(
            super::super::edit::syntax("x.cshtml", "<style>@media screen {}</style>", &ctx)
                .unwrap_err()
                .0,
            "RAZOR_UNESCAPED_MEDIA"
        );
    }
}

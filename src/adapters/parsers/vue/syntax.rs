//! Conservative Vue SFC editing validation. This validates inline script blocks
//! with the existing JS/TS parser; it does not claim to validate template/style.
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};

pub(in crate::adapters) struct Tag<'a> {
    pub name: &'a str,
    pub attrs: &'a str,
    pub end: usize,
    pub closing: bool,
    pub self_closing: bool,
}

pub(in crate::adapters) fn validate(
    path: &str,
    source: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    const MAX_SFC: usize = 2 * 1024 * 1024;
    if source.len() > MAX_SFC {
        return Err(("PARSER_LIMIT", "Vue component parser limited to 2 MiB"));
    }
    let mut cursor = 0;
    let mut scripts_checked = 0usize;
    let mut external_scripts = 0usize;
    let mut last_check = 0usize;
    while cursor < source.len() {
        if cursor.saturating_sub(last_check) >= 32 * 1024 {
            context.check()?;
            last_check = cursor;
        }
        let Some(start) = source[cursor..].find('<').map(|n| cursor + n) else {
            break;
        };
        if source[start..].starts_with("<!--") {
            cursor = source[start + 4..]
                .find("-->")
                .map(|n| start + 4 + n + 3)
                .ok_or(("SYNTAX_ERROR", "Unterminated Vue SFC comment"))?;
            continue;
        }
        let Some(tag) = parse_tag(source, start) else {
            cursor = start + 1;
            continue;
        };
        cursor = tag.end;
        if tag.closing {
            continue;
        }
        match tag.name.to_ascii_lowercase().as_str() {
            "template" if !tag.self_closing => cursor = skip_template(source, cursor, context)?,
            "script" if tag.self_closing => {
                if has_attribute(tag.attrs, "src") {
                    external_scripts += 1;
                } else {
                    scripts_checked += 1;
                }
            }
            "script" => {
                let (body_end, after) = closing_tag(source, cursor, "script")
                    .ok_or(("SYNTAX_ERROR", "Vue script block is not closed"))?;
                let body = &source[cursor..body_end];
                if has_attribute(tag.attrs, "src") {
                    external_scripts += 1;
                } else {
                    let lang = attribute(tag.attrs, "lang")
                        .unwrap_or("js")
                        .to_ascii_lowercase();
                    let virtual_path = match lang.as_str() {
                        "" | "js" | "javascript" => "component.vue.js",
                        "ts" | "typescript" => "component.vue.ts",
                        "jsx" => "component.vue.jsx",
                        "tsx" => "component.vue.tsx",
                        _ => {
                            return Err((
                                "PARSER_UNAVAILABLE",
                                "Vue script lang is not supported; use js, ts, jsx or tsx",
                            ));
                        }
                    };
                    crate::adapters::parsers::ecmascript::syntax::validate_syntax(
                        virtual_path,
                        body,
                        context,
                    )?;
                    scripts_checked += 1;
                }
                cursor = after;
            }
            // Styles and custom blocks are not JavaScript. Skip their raw body so
            // tags in CSS, JSON or i18n content cannot be mistaken for SFC blocks.
            name if !tag.self_closing => {
                if let Some((_, after)) = closing_tag(source, cursor, name) {
                    cursor = after;
                }
            }
            _ => {}
        }
    }
    context.check()?;
    let status = if scripts_checked == 0 {
        "not_checked"
    } else {
        "partial"
    };
    Ok(
        json!({"status":status,"language":"vue","validation":"embedded_scripts_only",
        "scriptsChecked":scripts_checked,"externalScriptsNotChecked":external_scripts,
        "templateAndStyles":"not_checked","componentPath":path}),
    )
}

pub(in crate::adapters) fn parse_tag(source: &str, start: usize) -> Option<Tag<'_>> {
    let bytes = source.as_bytes();
    if bytes.get(start) != Some(&b'<') {
        return None;
    }
    let mut name_start = start + 1;
    let closing = bytes.get(name_start) == Some(&b'/');
    if closing {
        name_start += 1;
    }
    if !bytes.get(name_start).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut name_end = name_start + 1;
    while bytes
        .get(name_end)
        .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b':' | b'_' | b'.'))
    {
        name_end += 1;
    }
    let mut quote = None;
    let mut end = name_end;
    while end < bytes.len() {
        let b = bytes[end];
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
        } else if matches!(b, b'\'' | b'"') {
            quote = Some(b);
        } else if b == b'>' {
            let before = bytes[name_end..end]
                .iter()
                .rposition(|b| !b.is_ascii_whitespace())
                .map(|n| name_end + n);
            return Some(Tag {
                name: &source[name_start..name_end],
                attrs: &source[name_end..end],
                end: end + 1,
                closing,
                self_closing: !closing && before.is_some_and(|n| bytes[n] == b'/'),
            });
        }
        end += 1;
    }
    None
}

pub(in crate::adapters) fn closing_tag(
    source: &str,
    from: usize,
    wanted: &str,
) -> Option<(usize, usize)> {
    let mut cursor = from;
    while let Some(relative) = source[cursor..].find('<') {
        let start = cursor + relative;
        if source[start..].starts_with("<!--") {
            cursor = source[start + 4..].find("-->").map(|n| start + 4 + n + 3)?;
            continue;
        }
        if let Some(tag) = parse_tag(source, start) {
            if tag.closing && tag.name.eq_ignore_ascii_case(wanted) {
                return Some((start, tag.end));
            }
            cursor = tag.end;
        } else {
            cursor = start + 1;
        }
    }
    None
}

fn skip_template(source: &str, from: usize, context: &OperationContext) -> Result<usize, Failure> {
    let mut depth = 1usize;
    let mut cursor = from;
    let mut last_check = from;
    while cursor < source.len() {
        if cursor.saturating_sub(last_check) >= 32 * 1024 {
            context.check()?;
            last_check = cursor;
        }
        let next_tag = source[cursor..].find('<').map(|n| cursor + n);
        let next_interpolation = source[cursor..].find("{{").map(|n| cursor + n);
        let Some(start) = [next_tag, next_interpolation].into_iter().flatten().min() else {
            break;
        };
        if source[start..].starts_with("{{") {
            cursor = skip_interpolation(source, start + 2)
                .ok_or(("SYNTAX_ERROR", "Vue template interpolation is not closed"))?;
            continue;
        }
        if source[start..].starts_with("<!--") {
            cursor = source[start + 4..]
                .find("-->")
                .map(|n| start + 4 + n + 3)
                .ok_or(("SYNTAX_ERROR", "Unterminated Vue template comment"))?;
            continue;
        }
        if let Some(tag) = parse_tag(source, start) {
            if tag.name.eq_ignore_ascii_case("template") {
                if tag.closing {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(tag.end);
                    }
                } else if !tag.self_closing {
                    depth += 1;
                }
            }
            cursor = tag.end;
        } else {
            cursor = start + 1;
        }
    }
    Err(("SYNTAX_ERROR", "Vue template block is not closed"))
}

pub(in crate::adapters) fn skip_interpolation(source: &str, mut cursor: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut quote = None;
    let mut escaped = false;
    let mut braces = 0usize;
    let mut line_comment = false;
    let mut block_comment = false;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if line_comment {
            if byte == b'\n' {
                line_comment = false;
            }
        } else if block_comment {
            if byte == b'*' && bytes.get(cursor + 1) == Some(&b'/') {
                block_comment = false;
                cursor += 1;
            }
        } else if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == active_quote {
                quote = None;
            }
        } else if byte == b'/' && bytes.get(cursor + 1) == Some(&b'/') {
            line_comment = true;
            cursor += 1;
        } else if byte == b'/' && bytes.get(cursor + 1) == Some(&b'*') {
            block_comment = true;
            cursor += 1;
        } else if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
        } else if byte == b'{' {
            braces += 1;
        } else if byte == b'}' {
            if braces > 0 {
                braces -= 1;
            } else if bytes.get(cursor + 1) == Some(&b'}') {
                return Some(cursor + 2);
            }
        }
        cursor += 1;
    }
    None
}

pub(in crate::adapters) fn has_attribute(attrs: &str, wanted: &str) -> bool {
    attribute(attrs, wanted).is_some()
}

pub(in crate::adapters) fn attribute<'a>(attrs: &'a str, wanted: &str) -> Option<&'a str> {
    let bytes = attrs.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        while cursor < bytes.len() && (bytes[cursor].is_ascii_whitespace() || bytes[cursor] == b'/')
        {
            cursor += 1;
        }
        let start = cursor;
        while cursor < bytes.len()
            && !bytes[cursor].is_ascii_whitespace()
            && !matches!(bytes[cursor], b'=' | b'/')
        {
            cursor += 1;
        }
        if start == cursor {
            cursor += 1;
            continue;
        }
        let name = &attrs[start..cursor];
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let value = if bytes.get(cursor) == Some(&b'=') {
            cursor += 1;
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if cursor < bytes.len() && matches!(bytes[cursor], b'\'' | b'"') {
                let quote = bytes[cursor];
                cursor += 1;
                let start = cursor;
                while cursor < bytes.len() && bytes[cursor] != quote {
                    cursor += 1;
                }
                let value = &attrs[start..cursor];
                cursor += usize::from(cursor < bytes.len());
                value
            } else {
                let start = cursor;
                while cursor < bytes.len() && !bytes[cursor].is_ascii_whitespace() {
                    cursor += 1;
                }
                &attrs[start..cursor]
            }
        } else {
            ""
        };
        if name.eq_ignore_ascii_case(wanted) {
            return Some(value);
        }
    }
    None
}

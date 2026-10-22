//! A bounded selector subset over original attributes, preserving Angular spelling.
use harness_domain::Failure;
const INVALID: Failure = (
    "ARGUMENT",
    "HTML selector supports tag, #id, .class, [attribute], [attribute=value] and descendants; no pseudo-classes, groups, escapes or sibling/child combinators",
);

#[derive(Default)]
pub(super) struct Compound {
    tag: Option<String>,
    attrs: Vec<(String, Option<String>, bool)>, // class requires whitespace membership
}

impl Compound {
    pub(super) fn matches(&self, tag: &str, attrs: &[(&str, Option<&str>)]) -> bool {
        self.tag
            .as_ref()
            .is_none_or(|name| name.eq_ignore_ascii_case(tag))
            && self.attrs.iter().all(|(key, expected, class)| {
                attrs.iter().any(|(name, value)| {
                    key.eq_ignore_ascii_case(name)
                        && match expected {
                            None => true,
                            Some(expected) if *class => value.is_some_and(|value| {
                                value.split_ascii_whitespace().any(|part| part == expected)
                            }),
                            Some(expected) => value.is_some_and(|value| value == expected),
                        }
                })
            })
    }
}

pub(super) fn parse(query: &str) -> Result<Vec<Compound>, Failure> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut brackets = 0usize;
    let mut quote = 0;
    for (at, byte) in query.bytes().enumerate() {
        if byte == b'\\' {
            return Err(INVALID);
        }
        if quote != 0 {
            if byte == quote {
                quote = 0;
            }
        } else if brackets > 0 && matches!(byte, b'\'' | b'"') {
            quote = byte;
        } else if byte == b'[' {
            brackets += 1;
            if brackets > 1 {
                return Err(INVALID);
            }
        } else if byte == b']' {
            brackets = brackets.checked_sub(1).ok_or(INVALID)?;
        } else if brackets == 0 && byte.is_ascii_whitespace() {
            if start < at {
                parts.push(compound(&query[start..at])?);
            }
            start = at + 1;
        } else if brackets == 0 && matches!(byte, b'>' | b'+' | b'~' | b',' | b':') {
            return Err(INVALID);
        }
    }
    if quote != 0 || brackets != 0 {
        return Err(INVALID);
    }
    if start < query.len() {
        parts.push(compound(&query[start..])?);
    }
    if parts.is_empty() || parts.len() > 32 {
        return Err(INVALID);
    }
    Ok(parts)
}

fn compound(query: &str) -> Result<Compound, Failure> {
    let bytes = query.as_bytes();
    let mut result = Compound::default();
    let mut at = 0;
    if bytes.first() == Some(&b'*') {
        at = 1;
    } else if !bytes
        .first()
        .is_some_and(|b| matches!(b, b'#' | b'.' | b'['))
    {
        let end = identifier(bytes, at);
        if end == at {
            return Err(INVALID);
        }
        result.tag = Some(query[at..end].to_owned());
        at = end;
    }
    while at < bytes.len() {
        let marker = bytes[at];
        at += 1;
        match marker {
            b'#' | b'.' => {
                let end = identifier(bytes, at);
                if end == at {
                    return Err(INVALID);
                }
                result.attrs.push((
                    if marker == b'#' { "id" } else { "class" }.into(),
                    Some(query[at..end].into()),
                    marker == b'.',
                ));
                at = end;
            }
            b'[' => {
                let start = at;
                let mut quote = 0;
                while at < bytes.len() {
                    if quote != 0 {
                        if bytes[at] == quote {
                            quote = 0;
                        }
                    } else if matches!(bytes[at], b'\'' | b'"') {
                        quote = bytes[at];
                    } else if bytes[at] == b']' {
                        break;
                    }
                    at += 1;
                }
                if at == bytes.len() {
                    return Err(INVALID);
                }
                let attribute = query[start..at].trim();
                let (key, value) = attribute
                    .split_once('=')
                    .map_or((attribute, None), |(key, value)| {
                        (key.trim(), Some(value.trim()))
                    });
                if key.is_empty()
                    || !key
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':'))
                {
                    return Err(INVALID);
                }
                let value = value
                    .map(|value| {
                        if value.starts_with(['\'', '"']) {
                            if value.len() < 2
                                || value.as_bytes().last() != value.as_bytes().first()
                            {
                                return Err(INVALID);
                            }
                            let body = &value[1..value.len() - 1];
                            if body.as_bytes().contains(&value.as_bytes()[0]) {
                                return Err(INVALID);
                            }
                            Ok(body)
                        } else {
                            if value.is_empty()
                                || value
                                    .bytes()
                                    .any(|b| b.is_ascii_whitespace() || matches!(b, b'\'' | b'"'))
                            {
                                return Err(INVALID);
                            }
                            Ok(value)
                        }
                    })
                    .transpose()?;
                result
                    .attrs
                    .push((key.into(), value.map(Into::into), false));
                at += 1;
            }
            _ => return Err(INVALID),
        }
        if result.attrs.len() > 32 {
            return Err(INVALID);
        }
    }
    Ok(result)
}
fn identifier(bytes: &[u8], mut at: usize) -> usize {
    while at < bytes.len()
        && (bytes[at].is_ascii_alphanumeric()
            || bytes[at] >= 128
            || matches!(bytes[at], b'_' | b'-'))
    {
        at += 1;
    }
    at
}

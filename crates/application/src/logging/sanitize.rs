use serde_json::{Value, json};

fn sensitive(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "password",
        "passwd",
        "pwd=",
        "token",
        "secret",
        "credential",
        "database_url",
        "authorization",
        "api_key",
        "apikey",
        "private key",
        "://",
    ]
    .iter()
    .any(|key| lower.contains(key))
}

pub(super) fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1000
        && !sensitive(path)
        && !path.starts_with(['/', '\\'])
        && !path.contains(':')
        && !path.chars().any(char::is_control)
        && !path
            .replace('\\', "/")
            .split('/')
            .any(|part| part == ".." || part.starts_with(".env"))
}

pub(super) fn sanitize_text(text: &str) -> String {
    let bounded: String = text
        .chars()
        .rev()
        .take(4000)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    bounded
        .lines()
        .map(|line| {
            if sensitive(line) {
                "[redacted sensitive diagnostic line]".to_owned()
            } else {
                line.chars()
                    .filter(|c| !c.is_control() || *c == '\t')
                    .collect()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn sanitize_value(value: &Value) -> Value {
    match value {
        Value::String(s) => json!(sanitize_text(s)),
        Value::Array(items) => Value::Array(items.iter().take(16).map(sanitize_value).collect()),
        Value::Object(items) => Value::Object(
            items
                .iter()
                .take(4)
                .map(|(k, v)| (k.clone(), sanitize_value(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

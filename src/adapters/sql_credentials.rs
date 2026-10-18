//! Resolve one named secret without mutating the process environment or evaluating dotenv code.
use super::project::Project;
use harness_domain::{Failure, execution::OperationContext};

const ENV_FILE: Failure = (
    "PG_ENV_FILE",
    "Cannot load the selected variable from the project dotenv file; check its path, UTF-8, size and single-line syntax",
);
pub(super) fn resolve(
    project: &Project,
    name: &str,
    file: Option<&str>,
    context: &OperationContext,
) -> Result<String, Failure> {
    context.check()?;
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => return Ok(value),
        Ok(_) | Err(std::env::VarError::NotUnicode(_)) => {
            return Err((
                "PG_CREDENTIALS",
                "The selected process environment variable is empty or invalid",
            ));
        }
        Err(std::env::VarError::NotPresent) => {}
    }
    if let Some(file) = file {
        let target = project.target_optional(file).map_err(|_| ENV_FILE)?;
        if target.exists() {
            let bytes =
                project
                    .bytes_with_limit(file, context, 64 * 1024)
                    .map_err(|(code, _)| {
                        if matches!(code, "CANCELLED" | "TIMEOUT") {
                            (code, "Credential lookup interrupted")
                        } else {
                            ENV_FILE
                        }
                    })?;
            let text = std::str::from_utf8(&bytes).map_err(|_| ENV_FILE)?;
            if let Some(value) = parse(text, name)? {
                return Ok(value);
            }
        }
    }
    Err((
        "PG_CREDENTIALS",
        "Set the selected connection variable in the MCP process environment or its configured project dotenv file",
    ))
}

fn parse(text: &str, name: &str) -> Result<Option<String>, Failure> {
    let mut found = None;
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        if line.starts_with('#') {
            continue;
        }
        let Some((key, raw)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != name {
            continue;
        }
        if found.is_some() {
            return Err(ENV_FILE);
        }
        let raw = raw.trim();
        let value = if raw.starts_with(['\'', '"']) {
            let quote = raw.chars().next().expect("quoted value");
            let mut value = String::new();
            let mut chars = raw[1..].char_indices();
            let mut closed = false;
            while let Some((index, c)) = chars.next() {
                if c == quote {
                    let suffix = raw[index + 2..].trim_start();
                    if !suffix.is_empty() && !suffix.starts_with('#') {
                        return Err(ENV_FILE);
                    }
                    closed = true;
                    break;
                }
                if c == '\\' && quote == '"' {
                    let (_, escaped) = chars.next().ok_or(ENV_FILE)?;
                    value.push(match escaped {
                        '\\' => '\\',
                        '"' => '"',
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        _ => return Err(ENV_FILE),
                    });
                } else {
                    value.push(c);
                }
            }
            if !closed {
                return Err(ENV_FILE);
            }
            value
        } else {
            let end = raw
                .char_indices()
                .find(|(i, c)| *c == '#' && *i > 0 && raw[..*i].ends_with(char::is_whitespace))
                .map(|(i, _)| i)
                .unwrap_or(raw.len());
            raw[..end].trim_end().to_owned()
        };
        if value.is_empty() || value.contains("${") || value.contains(['\r', '\n', '\0']) {
            return Err(ENV_FILE);
        }
        found = Some(value);
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dotenv_selects_only_one_variable_and_handles_quotes_comments_and_equals() {
        let text = "\u{feff}OTHER=ignored\nexport DSN = \"postgresql://test:pass@localhost/db?application_name=a=b\" # comment\n";
        assert_eq!(
            parse(text, "DSN").unwrap().unwrap(),
            "postgresql://test:pass@localhost/db?application_name=a=b"
        );
        assert_eq!(
            parse("DSN='value#literal'", "DSN").unwrap().unwrap(),
            "value#literal"
        );
        assert_eq!(
            parse("DSN=value # comment", "DSN").unwrap().unwrap(),
            "value"
        );
        assert!(parse(text, "ABSENT").unwrap().is_none());
    }
    #[test]
    fn dotenv_rejects_ambiguous_or_executable_expansion_syntax() {
        for text in [
            "DSN=a\nDSN=b",
            "DSN=\"unfinished",
            "DSN=${PASSWORD}",
            "DSN=",
            "DSN='ok' extra",
        ] {
            assert_eq!(parse(text, "DSN").unwrap_err().0, "PG_ENV_FILE");
        }
    }
}

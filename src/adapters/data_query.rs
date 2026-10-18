//! Bounded JSON projection. Redact before evaluating queries or storing results.

use super::project::Project;
use harness_domain::{Failure, contracts::DataQueryArgs, execution::OperationContext};
use serde_json::{Value, json};
use std::path::Path;

pub(super) fn redact(
    value: &mut Value,
    depth: usize,
    visited: &mut usize,
    context: &OperationContext,
) -> Result<(), Failure> {
    *visited += 1;
    if depth > 64 || *visited > 10000 {
        return Err(("QUERY_LIMIT", "JSON exceeds depth/node budget"));
    }
    context.check()?;
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                let key: String = key
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect();
                if [
                    "password",
                    "passwd",
                    "pwd",
                    "secret",
                    "token",
                    "apikey",
                    "authorization",
                    "connectionstring",
                    "credential",
                    "privatekey",
                ]
                .iter()
                .any(|s| key.contains(s))
                {
                    *value = json!("[REDACTED]");
                } else {
                    redact(value, depth + 1, visited, context)?;
                }
            }
        }
        Value::Array(items) => {
            for value in items {
                redact(value, depth + 1, visited, context)?;
            }
        }
        Value::String(text) => {
            let lower = text.to_ascii_lowercase();
            if [
                "password=",
                "password:",
                "pwd=",
                "bearer ",
                "authorization:",
                "-----begin private key",
                "-----begin rsa private key",
            ]
            .iter()
            .any(|needle| lower.contains(needle))
            {
                *text = "[REDACTED]".into();
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn handle_data_query(
    project: &Project,
    args: &DataQueryArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;

    if args.query.is_empty() || args.query.len() > 4096 {
        return Err(("ARGUMENT", "query es obligatorio (1–4096 caracteres)"));
    }

    if usize::from(args.path.is_some()) + usize::from(args.data.is_some()) != 1
        || args.result_ref.is_some()
    {
        return Err((
            "ARGUMENT",
            "Use exactly one of path, data or resultRef (resolved by application)",
        ));
    }

    let max_chars = args.max_chars.unwrap_or(8000);
    if !(500..=32000).contains(&max_chars) {
        return Err(("ARGUMENT", "maxChars: 500–32000"));
    }

    let mut input_value: Value = if let Some(path) = &args.path {
        context.check()?;
        let bytes = project.bytes_with_context(path, context)?;
        let ext = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        match ext.as_str() {
            "json" => {
                let text = std::str::from_utf8(&bytes)
                    .map_err(|_| ("ENCODING", "JSON no es UTF-8 válido"))?;
                let clean = text.strip_prefix('\u{feff}').unwrap_or(text);
                serde_json::from_str::<Value>(clean)
                    .map_err(|_| ("SYNTAX_ERROR", "JSON inválido en el archivo indicado"))?
            }
            _ => {
                return Err((
                    "UNSUPPORTED",
                    "harness_data_query soporta archivos JSON en este release; para datos arbitrarios pasar campo 'data'",
                ));
            }
        }
    } else if let Some(data) = &args.data {
        data.clone()
    } else {
        return Err(("ARGUMENT", "Indicar path o data"));
    };

    context.check()?;

    if serde_json::to_vec(&input_value)
        .map_err(|_| ("QUERY_LIMIT", "JSON cannot be measured"))?
        .len()
        > 1024 * 1024
    {
        return Err(("QUERY_LIMIT", "JSON input exceeds 1 MiB"));
    }
    redact(&mut input_value, 0, &mut 0, context)?;

    // Compile JMESPath expression
    let expr = jmespath::compile(&args.query).map_err(|_| {
        (
            "QUERY_SYNTAX",
            "Error en la sintaxis de la consulta JMESPath",
        )
    })?;

    // Search input value
    let result = expr
        .search(&input_value)
        .map_err(|_| ("QUERY_EVAL", "Error al evaluar la consulta JMESPath"))?;
    context.check()?;

    let mut result_json: Value = serde_json::to_value(&*result).map_err(|_| {
        (
            "INTERNAL",
            "No se pudo serializar el resultado de la consulta",
        )
    })?;
    redact(&mut result_json, 0, &mut 0, context)?;

    let serialized = serde_json::to_string(&result_json).unwrap_or_default();
    let truncated = harness_domain::results::utf16_len(&serialized) > max_chars;

    let response_data = if truncated {
        json!({
            "truncated": true,
            "preview": harness_domain::results::prefix(&serialized, max_chars),
            "_detail": {"result": result_json},
            "redaction": "sensitive-key-heuristic-v1",
            "message": "El resultado excede maxChars; refine la consulta agregando slice [0:10] o proyecciones más específicas"
        })
    } else {
        json!({
            "truncated": false,
            "redaction": "sensitive-key-heuristic-v1",
            "result": result_json
        })
    };

    Ok(response_data)
}

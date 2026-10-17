//! Ultra-fast embedded JMESPath data query adapter.
//! Queries, slices, filters and projects structured data (JSON, TOML) in memory in < 0.1ms without external processes.

use super::project::Project;
use harness_domain::{Failure, contracts::DataQueryArgs, execution::OperationContext};
use serde_json::{Value, json};
use std::path::Path;

pub fn handle_data_query(
    project: &Project,
    args: &DataQueryArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;

    if args.query.is_empty() || args.query.len() > 4096 {
        return Err(("ARGUMENT", "query es obligatorio (1–4096 caracteres)"));
    }

    if args.path.is_none() && args.data.is_none() {
        return Err(("ARGUMENT", "Indicar path o data"));
    }

    let max_chars = args.max_chars.unwrap_or(8000);
    if !(500..=32000).contains(&max_chars) {
        return Err(("ARGUMENT", "maxChars: 500–32000"));
    }

    let input_value: Value = if let Some(path) = &args.path {
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

    // Compile JMESPath expression
    let expr = jmespath::compile(&args.query)
        .map_err(|_| ("QUERY_SYNTAX", "Error en la sintaxis de la consulta JMESPath"))?;

    // Search input value
    let result = expr
        .search(&input_value)
        .map_err(|_| ("QUERY_EVAL", "Error al evaluar la consulta JMESPath"))?;

    let result_json: Value = serde_json::to_value(&*result)
        .map_err(|_| ("INTERNAL", "No se pudo serializar el resultado de la consulta"))?;

    let serialized = serde_json::to_string(&result_json).unwrap_or_default();
    let truncated = serialized.len() > max_chars;

    let response_data = if truncated {
        json!({
            "truncated": true,
            "query": args.query,
            "preview": &serialized[..max_chars],
            "message": "El resultado excede maxChars; refine la consulta agregando slice [0:10] o proyecciones más específicas"
        })
    } else {
        json!({
            "truncated": false,
            "query": args.query,
            "result": result_json
        })
    };

    Ok(response_data)
}

//! Native fast batch syntax diagnostics adapter.
//! Validates Rust (syn), JS/TS (oxc), JSON, and skips unsupported or clean template/text files.
//! Replaces diagnostics.cjs without Node or external processes.

use super::project::Project;
use harness_domain::{
    Failure,
    contracts::{DiagnosticsArgs, GitStatusArgs},
    execution::OperationContext,
    hash,
};
use serde_json::{Value, json};
use std::path::Path;

fn validate_syntax_single(path: &str, source: &str, context: &OperationContext) -> (Value, usize) {
    match Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" => match super::rust_syntax::with_ast(source, context, |_, _| Ok(())) {
            Ok(()) => (
                json!({
                    "status": "checked",
                    "language": "rust",
                    "errors": 0,
                    "diagnostics": []
                }),
                0,
            ),
            Err((code, msg)) => (
                json!({
                    "status": "checked",
                    "language": "rust",
                    "errors": 1,
                    "diagnostics": [{"code": code, "message": msg}]
                }),
                1,
            ),
        },
        "js" | "mjs" | "cjs" | "jsx" => {
            match super::ts_syntax::validate_syntax(path, source, context) {
                Ok(()) => (
                    json!({
                        "status": "checked",
                        "language": "javascript",
                        "errors": 0,
                        "diagnostics": []
                    }),
                    0,
                ),
                Err((code, msg)) => (
                    json!({
                        "status": "checked",
                        "language": "javascript",
                        "errors": 1,
                        "diagnostics": [{"code": code, "message": msg}]
                    }),
                    1,
                ),
            }
        }
        "ts" | "mts" | "cts" | "tsx" => {
            match super::ts_syntax::validate_syntax(path, source, context) {
                Ok(()) => (
                    json!({
                        "status": "checked",
                        "language": "typescript",
                        "errors": 0,
                        "diagnostics": []
                    }),
                    0,
                ),
                Err((code, msg)) => (
                    json!({
                        "status": "checked",
                        "language": "typescript",
                        "errors": 1,
                        "diagnostics": [{"code": code, "message": msg}]
                    }),
                    1,
                ),
            }
        }
        "vue" => match super::vue_syntax::validate(path, source, context) {
            Ok(mut result) => {
                result["errors"] = json!(0);
                result["diagnostics"] = json!([{"code":"VUE_PARTIAL_VALIDATION","message":"Inline JS/TS scripts checked; template and style blocks are not checked"}]);
                (result, 0)
            }
            Err((code, msg)) => (
                json!({"status":"checked","language":"vue","errors":1,"diagnostics":[{"code":code,"message":msg}]}),
                1,
            ),
        },
        "astro" => match super::astro_syntax::validate(path, source, context) {
            Ok(mut result) => {
                result["errors"] = json!(0);
                result["diagnostics"] = json!([{"code":"ASTRO_PARTIAL_VALIDATION","message":"Frontmatter TypeScript/JavaScript and inline scripts checked; template markup is not type-checked"}]);
                (result, 0)
            }
            Err((code, msg)) => (
                json!({"status":"checked","language":"astro","errors":1,"diagnostics":[{"code":code,"message":msg}]}),
                1,
            ),
        },
        "json" => {
            let clean = source.strip_prefix('\u{feff}').unwrap_or(source);
            match serde_json::from_str::<Value>(clean) {
                Ok(_) => (
                    json!({
                        "status": "checked",
                        "language": "json",
                        "errors": 0,
                        "diagnostics": []
                    }),
                    0,
                ),
                Err(err) => (
                    json!({
                        "status": "checked",
                        "language": "json",
                        "errors": 1,
                        "diagnostics": [{"code": "JSON_SYNTAX", "message": format!("{err}")}]
                    }),
                    1,
                ),
            }
        }
        "cs" | "cshtml" => match super::csharp_syntax::validate_syntax(path, source, context) {
            Ok(result) => (result, 0),
            Err((code, msg)) => (
                json!({
                    "status": "checked",
                    "language": if path.ends_with(".cshtml") { "razor" } else { "csharp" },
                    "errors": 1,
                    "diagnostics": [{"code": code, "message": msg}]
                }),
                1,
            ),
        },
        "md" | "txt" | "toml" | "yaml" | "yml" => (
            json!({
                "status": "skipped",
                "reason": "unsupported_language"
            }),
            0,
        ),
        _ => (
            json!({
                "status": "skipped",
                "reason": "unsupported_language"
            }),
            0,
        ),
    }
}

pub fn handle_diagnostics(
    project: &Project,
    args: &DiagnosticsArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;

    if let Some(files) = &args.files {
        if files.is_empty() || files.len() > 50 {
            return Err(("ARGUMENT", "files: 1–50 archivos relativos explícitos"));
        }
        for file in files {
            if file.is_empty() || file.len() > 2048 {
                return Err(("ARGUMENT", "files: 1–50 archivos relativos explícitos"));
            }
        }
    }

    let max_files = args.max_files.unwrap_or(20);
    if !(1..=50).contains(&max_files) {
        return Err(("ARGUMENT", "maxFiles: 1–50"));
    }

    if let Some(scope) = &args.build_scope {
        let is_valid = scope.ends_with(".csproj") || scope.ends_with(".sln");
        if !is_valid {
            return Err(("ARGUMENT", "buildScope requiere .csproj o .sln"));
        }
    }

    let (candidates, selection_type) = if let Some(files) = &args.files {
        (files.clone(), "explicit files")
    } else {
        // Collect from git status
        let status = super::git::status(
            project,
            &GitStatusArgs {
                include_untracked: Some(false),
                offset: None,
                limit: Some(100),
                session: None,
            },
            context,
        );
        match status {
            Ok(val) => {
                let mut list = Vec::new();
                if let Some(entries) = val["entries"].as_array() {
                    for entry in entries {
                        if let Some(p) = entry["path"].as_str() {
                            let xy = entry["xy"].as_str().unwrap_or("");
                            if !xy.contains('D') {
                                list.push(p.to_string());
                            }
                        }
                    }
                }
                (
                    list,
                    "tracked Git changes (staged and unstaged); deleted/untracked files excluded",
                )
            }
            Err(_) => (
                Vec::new(),
                "tracked Git changes (staged and unstaged); deleted/untracked files excluded",
            ),
        }
    };

    let total_candidates = candidates.len();
    let selection_truncated = total_candidates > max_files;
    let selected_files: Vec<String> = candidates.into_iter().take(max_files).collect();

    let mut files_out = Vec::new();
    let mut total_errors = 0;

    for rel_path in &selected_files {
        context.check()?;
        let target = project.target(rel_path)?;

        if target.is_dir() {
            return Err(("PATH", "Se requieren archivos, no carpetas"));
        }

        let bytes = match project.bytes_with_context(rel_path, context) {
            Ok(b) => b,
            Err((code, _)) => {
                files_out.push(json!({
                    "path": rel_path,
                    "status": "skipped",
                    "reason": code
                }));
                continue;
            }
        };

        let file_hash = hash(&bytes);
        let source_str = match std::str::from_utf8(&bytes) {
            Ok(s) => s,
            Err(_) => {
                files_out.push(json!({
                    "path": rel_path,
                    "hash": file_hash,
                    "status": "skipped",
                    "reason": "invalid_utf8"
                }));
                continue;
            }
        };

        let (mut analysis, err_count) = validate_syntax_single(rel_path, source_str, context);
        total_errors += err_count;

        let obj = analysis.as_object_mut().unwrap();
        obj.insert("path".to_string(), json!(rel_path));
        obj.insert("hash".to_string(), json!(file_hash));
        files_out.push(analysis);
    }

    let complete = !selection_truncated && files_out.iter().all(|f| f["status"] == "checked");
    let passed = total_errors == 0
        && files_out
            .iter()
            .all(|f| f["status"] != "failed" && f["status"] != "stale");

    let status_str = if !passed {
        "issues_found"
    } else if files_out.is_empty() {
        "no_files_selected"
    } else if complete {
        "no_errors_in_checked_scope"
    } else {
        "incomplete"
    };

    let mut result = json!({
        "pass": passed && complete,
        "status": status_str,
        "complete": complete,
        "errors": total_errors,
        "selection": selection_type,
        "totalCandidates": total_candidates,
        "selectionTruncated": selection_truncated,
        "files": files_out,
        "build": Value::Null,
        "coverage": "syntax-only by default; templates are partial; no type checking, lint or semantic analysis; bounded by path and exact content hash"
    });

    if !passed {
        result["code"] = json!("DIAGNOSTICS_FAILED");
        result["message"] =
            json!("Se detectaron errores o comprobaciones fallidas; revisar cobertura");
    } else if !complete {
        result["code"] = json!("DIAGNOSTICS_INCOMPLETE");
        result["message"] = json!(
            "Cobertura parcial, archivos omitidos o selección recortada; no afirmar validación completa"
        );
    }

    Ok(result)
}

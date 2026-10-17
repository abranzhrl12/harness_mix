use super::{edit_commit, project::Project};
use harness_domain::{Failure, contracts::ReplaceArgs, editing, execution::OperationContext, hash};
use serde_json::{Value, json};
use std::path::Path;

fn syntax(path: &str, source: &str, context: &OperationContext) -> Result<Value, Failure> {
    match Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" => {
            super::rust_syntax::with_ast(source, context, |_, _| Ok(()))?;
            Ok(json!({"status":"valid","language":"rust","validation":"syntax_only"}))
        }
        "js" | "mjs" | "cjs" | "jsx" => {
            super::ts_syntax::validate_syntax(path, source, context)?;
            Ok(json!({"status":"valid","language":"javascript","validation":"syntax_only"}))
        }
        "ts" | "mts" | "cts" | "tsx" => {
            super::ts_syntax::validate_syntax(path, source, context)?;
            Ok(json!({"status":"valid","language":"typescript","validation":"syntax_only"}))
        }
        "json" => {
            serde_json::from_str::<Value>(source.strip_prefix('\u{feff}').unwrap_or(source))
                .map_err(|_| ("SYNTAX_ERROR", "Candidate JSON invalid; original untouched"))?;
            Ok(json!({"status":"valid","language":"json","validation":"syntax_only"}))
        }
        "md" | "txt" => {
            Ok(json!({"status":"not_applicable","language":"text","validation":"none"}))
        }
        _ => Err((
            "PARSER_UNAVAILABLE",
            "Editing this format is not migrated; original untouched",
        )),
    }
}

pub(super) fn replace(
    project: &Project,
    args: &ReplaceArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    context.check()?;
    let target = edit_commit::target(project, &args.path)?;
    let _pins = edit_commit::pin_parents(project, &target)?;
    let bytes = project.bytes_with_context(&args.path, context)?;
    let updated = editing::prepare(&bytes, args)?;
    let syntax = syntax(&args.path, &updated, context)?;
    context.check()?;
    let mut result = edit_commit::publish(project, args, &target, &updated, context)?;
    result["syntax"] = syntax;
    if result["changed"] == true {
        // Commit has happened: cancellation must not hide this fact.
        let observed = project.bytes_with_context(&args.path, &OperationContext::default());
        let expected = hash(updated.as_bytes());
        let verified = observed.is_ok_and(|bytes| hash(&bytes) == expected);
        result["hash"] = json!(expected);
        result["verified"] = json!(verified);
        if !verified {
            result["ok"] = json!(false);
            result["diagnostics"] = json!([{"code":"EDIT_POSTCHECK","message":"Publication completed but postcheck failed; reread before retrying"}]);
        }
    }
    Ok(result)
}

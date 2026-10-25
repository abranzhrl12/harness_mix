//! Native command used by the project's Antigravity PreToolUse hook.
use std::io::{self, Read};

const NATIVE_TOOL_MATCHER: &str = "^(view_file|view_file_outline|view_code_item|list_dir|find_by_name|grep_search|codebase_search|write_to_file|replace_file_content|multi_replace_file_content|run_command|read_file|search_files|edit_file)$";

pub fn print_profile() -> Result<(), String> {
    let profile = serde_json::json!({"harnessmix-native-tools": {"PreToolUse": [{
        "matcher":NATIVE_TOOL_MATCHER,
        "hooks":[{"type":"command", "command":"harnessmix hook deny-native", "timeout":10}]
    }]}});
    println!("{}", serde_json::to_string_pretty(&profile).map_err(|e| e.to_string())?);
    Ok(())
}

pub fn deny_native_tool() -> Result<(), String> {
    // The matcher selects native tools. Drain the event without logging its
    // arguments, which may contain project data or credentials.
    io::copy(&mut io::stdin().take(1024 * 1024), &mut io::sink())
        .map_err(|e| format!("No se pudo leer el evento del hook: {e}"))?;
    println!("{}", deny_response());
    Ok(())
}

fn deny_response() -> serde_json::Value {
    serde_json::json!({
        "decision":"deny",
        "reason":"HARNESSMIX_NATIVE_TOOL_DENIED: usa las herramientas MCP de Harness para leer, buscar, editar o verificar el proyecto."
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_tool_hook_is_a_hard_deny() {
        let result = deny_response();
        assert_eq!(result["decision"], "deny");
        assert!(result["reason"].as_str().unwrap().contains("Harness"));
    }

    #[test]
    fn matcher_blocks_native_file_tools_but_not_harness_mcp() {
        let matcher = regex::Regex::new(NATIVE_TOOL_MATCHER).unwrap();
        for tool in ["view_file", "view_file_outline", "run_command", "write_to_file"] {
            assert!(matcher.is_match(tool));
        }
        for tool in ["harness_harness_read", "mcp__harness__harness_bootstrap", "search_web"] {
            assert!(!matcher.is_match(tool));
        }
    }
}

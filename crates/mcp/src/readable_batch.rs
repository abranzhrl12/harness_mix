//! Readable batch presentation; canonical envelopes remain untouched.
use serde_json::Value;

pub(super) fn with_notices(mut text: String, envelope: &Value) -> String {
    let data = &envelope["data"];
    let mut seen = std::collections::BTreeSet::new();
    for diagnostics in [&envelope["diagnostics"], &data["warnings"]] {
        if let Some(items) = diagnostics.as_array() {
            for item in items {
                let code = item["code"].as_str().unwrap_or("NOTICE");
                let message = item
                    .as_str()
                    .or_else(|| item["message"].as_str())
                    .unwrap_or("");
                if seen.insert((code.to_owned(), message.to_owned())) {
                    text.push_str(&format!(
                        "\n> {}: {}\n",
                        super::clip(code, 80),
                        super::clip(message, 300)
                    ));
                }
            }
        }
    }
    let flags = [
        "truncated",
        "presentationTruncated",
        "contentTruncated",
        "hasMore",
        "snapshotOnly",
    ];
    let partial = flags.iter().any(|key| data[*key] == true)
        || data["scanComplete"] == false
        || data["complete"] == false
        || data.get("contextTruncation").is_some()
        || envelope.get("omitted").is_some_and(|value| match value {
            Value::Object(map) => !map.is_empty(),
            Value::Number(number) => number.as_u64().is_some_and(|n| n > 0),
            _ => false,
        });
    if data["previewTruncated"] == true {
        text.push_str("\n> Batch preview shortened to fit the response budget; full child results are recoverable from the batch detail.\n");
    } else if envelope["scope"]["tool"] == "harness_batch"
        && (data["truncated"] == true || envelope["omitted"]["truncated"] == true)
    {
        text.push_str("\n> Child previews were shortened to fit the response budget; full results are recoverable from the batch detail.\n");
    } else if partial {
        text.push_str(
            "\n> Partial output; check structuredContent for limits, omissions and continuation.\n",
        );
    }
    if let Some(reference) = envelope["detail"]["ref"].as_str() {
        text.push_str(&format!(
            "\n> detail.ref: {}\n",
            super::clip(reference, 100)
        ));
    }
    text
}

pub(super) fn render(envelope: &Value) -> Option<String> {
    render_inner(envelope, 0)
}

fn render_inner(envelope: &Value, depth: usize) -> Option<String> {
    let children = envelope["data"]["results"].as_array()?;
    if children.is_empty() && envelope["state"] != "ok" {
        return None;
    }
    let failed = children
        .iter()
        .filter(|child| child["state"] != "ok")
        .count();
    let mut text = format!("## Batch · {} results · {failed} failed\n", children.len());
    for (index, child) in children.iter().enumerate() {
        // Domain batches carry tool at the child root, rather than scope.tool.
        let tool = child["tool"]
            .as_str()
            .or_else(|| child["scope"]["tool"].as_str())
            .unwrap_or("result");
        let state = child["state"].as_str().unwrap_or("unknown");
        let target = child_target(tool, child);
        if let Some(target) = target.as_deref().filter(|s| !s.is_empty()) {
            text.push_str(&format!(
                "\n---\n\n### {} · {} · {} · `{}`\n\n",
                index + 1,
                super::clip(tool, 80),
                super::clip(state, 40),
                super::clip(target, 120)
            ));
        } else {
            text.push_str(&format!(
                "\n---\n\n### {} · {} · {}\n\n",
                index + 1,
                super::clip(tool, 80),
                super::clip(state, 40)
            ));
        }
        let body = if state == "ok" {
            if tool == "harness_batch" && depth < 4 {
                render_inner(child, depth + 1)
            } else {
                render_child_summary(tool, child)
            }
        } else {
            render_failed_child(child)
        };
        if let Some(body) = body {
            text.push_str(&with_notices(body, child));
        } else {
            // Preserve unfamiliar payloads and errors instead of guessing their meaning.
            // Pretty JSON is a readable fallback, never a JSON-escaped string.
            let body = serde_json::to_string_pretty(child).ok()?;
            let fence = super::fence_run(&body);
            text.push_str(&format!("{fence}json\n{body}\n{fence}\n"));
        }
    }
    Some(with_notices(text, envelope))
}

fn child_target(tool: &str, child: &Value) -> Option<String> {
    if let Some(target) = child["target"].as_str().filter(|s| !s.is_empty()) {
        return Some(target.to_owned());
    }
    let data = &child["data"];
    let scope_path = child["scope"]["path"].as_str().filter(|s| !s.is_empty());
    match tool {
        "harness_read" => {
            let path = data["path"].as_str().or(scope_path)?;
            if let Some(range) = data["range"].as_array() {
                if let (Some(start), Some(end)) = (range.first()?.as_u64(), range.get(1)?.as_u64())
                {
                    return Some(format!("{path}:{start}-{end}"));
                }
            }
            if let (Some(start), Some(end)) = (data["start"].as_u64(), data["end"].as_u64()) {
                return Some(format!("{path}:{start}-{end}"));
            }
            Some(path.to_owned())
        }
        "harness_symbols" => {
            let path = data["path"].as_str().or(scope_path)?;
            if let Some(name) = data["symbol"]["name"].as_str().filter(|s| !s.is_empty()) {
                Some(format!("{path}::{name}"))
            } else if data["items"].is_array() {
                Some(format!("outline {path}"))
            } else {
                Some(path.to_owned())
            }
        }
        "harness_search" => {
            if let Some(query) = data["query"].as_str().filter(|s| !s.is_empty()) {
                return Some(query.to_owned());
            }
            if let Some(selector) = data["selector"].as_str().filter(|s| !s.is_empty()) {
                return Some(selector.to_owned());
            }
            if let Some(path) = scope_path {
                return Some(path.to_owned());
            }
            None
        }
        "harness_explore" => {
            let file = data["file"]
                .as_str()
                .or_else(|| data["selection"]["filePath"].as_str())
                .or_else(|| {
                    data["definitions"]
                        .as_array()
                        .and_then(|defs| defs.first())
                        .and_then(|d| d["path"].as_str())
                });
            let symbol = data["symbol"]
                .as_str()
                .or_else(|| data["focus"]["symbol"].as_str())
                .or_else(|| {
                    data["definitions"]
                        .as_array()
                        .and_then(|defs| defs.first())
                        .and_then(|d| d["symbol"]["name"].as_str())
                });
            match (file, symbol) {
                (Some(f), Some(s)) if !f.is_empty() && !s.is_empty() => Some(format!("{f}::{s}")),
                (Some(f), _) if !f.is_empty() => Some(f.to_owned()),
                (_, Some(s)) if !s.is_empty() => Some(s.to_owned()),
                _ => None,
            }
        }
        "harness_git" => {
            if let Some(path) = data["path"].as_str().filter(|s| !s.is_empty()) {
                Some(format!("diff {path}"))
            } else if data["entries"].is_array() || data["branch"].is_string() {
                Some("status".to_owned())
            } else {
                scope_path.map(str::to_owned)
            }
        }
        "harness_sql" => {
            if let Some(name) = data["resolvedName"]
                .as_str()
                .or_else(|| data["name"].as_str())
                .filter(|s| !s.is_empty())
            {
                Some(name.to_owned())
            } else {
                data["engine"].as_str().map(str::to_owned)
            }
        }
        "harness_verify" => {
            let lang = data["language"]
                .as_str()
                .or_else(|| child["scope"]["language"].as_str());
            let scope = data["scope"].as_str().or(scope_path);
            match (lang, scope) {
                (Some(l), Some(s)) if !s.is_empty() => Some(format!("{l} ({s})")),
                (Some(l), _) => Some(l.to_owned()),
                (_, Some(s)) => Some(s.to_owned()),
                _ => None,
            }
        }
        _ => data["path"].as_str().or(scope_path).map(str::to_owned),
    }
}

fn render_child_summary(tool: &str, child: &Value) -> Option<String> {
    if tool == "harness_explore" {
        return render_explore_child(child);
    }
    if let Some(lean) = super::lean_channel(tool, child) {
        return Some(lean);
    }
    let data = &child["data"];
    match tool {
        "harness_search" => render_search_fallback(child),
        "harness_symbols" => render_symbols_fallback(child),
        "harness_sql" => render_sql_fallback(data),
        "harness_git" => render_git_fallback(data),
        "harness_verify" => render_verify_child(data),
        _ => None,
    }
}

fn render_explore_child(child: &Value) -> Option<String> {
    let data = &child["data"];
    let view = data["view"].as_str().unwrap_or("full");
    let index_freshness = data["indexFreshness"].as_str().unwrap_or("unknown");
    let file_freshness = data["fileFreshness"]["status"]
        .as_str()
        .or_else(|| data["fileFreshness"].as_str())
        .unwrap_or("unknown");

    if view == "impact" {
        let symbol = data["symbol"]
            .as_str()
            .or_else(|| data["focus"]["symbol"].as_str())
            .unwrap_or("target");
        let file = data["file"].as_str().unwrap_or("");
        let callers_count = data["counts"]["callers"]
            .as_u64()
            .or_else(|| data["callers"].as_array().map(|a| a.len() as u64))
            .unwrap_or(0);
        let direct_count = data["counts"]["directCallers"].as_u64().unwrap_or(0);
        let files_count = data["counts"]["files"]
            .as_u64()
            .or_else(|| data["files"].as_array().map(|a| a.len() as u64))
            .unwrap_or(0);
        let mut text = format!(
            "## Impact · {} {} · {} caller(s) ({} direct) across {} file(s) · freshness index={index_freshness} file={file_freshness}\n",
            super::clip(symbol, 80),
            if file.is_empty() {
                String::new()
            } else {
                format!("at {}", super::clip(file, 120))
            },
            callers_count,
            direct_count,
            files_count
        );
        if let Some(callers) = data["callers"]
            .as_array()
            .filter(|callers| !callers.is_empty())
        {
            text.push('\n');
            for caller in callers.iter().take(10) {
                let name = caller["symbol"].as_str().unwrap_or("?");
                let cfile = caller["file"].as_str().unwrap_or("?");
                let line = caller["declarationLine"].as_u64().unwrap_or(0);
                let kind = if caller["direct"] == true {
                    "direct"
                } else {
                    "indirect"
                };
                text.push_str(&format!(
                    "- `{}` ({}:{}) · {kind}\n",
                    super::clip(name, 80),
                    super::clip(cfile, 120),
                    line
                ));
            }
            let omitted = data["callersOmitted"].as_u64().unwrap_or(0) as usize
                + callers.len().saturating_sub(10);
            if omitted > 0 {
                text.push_str(&format!("\n> {omitted} more caller(s) omitted.\n"));
            }
        } else {
            text.push_str("\nNo indexed callers found.\n");
        }
        return Some(text);
    }

    let definitions = data["definitions"].as_array();
    let targets = definitions
        .map(|defs| defs.iter().filter(|d| d["role"] == "target").count())
        .unwrap_or(0);
    let related = definitions
        .map(|defs| defs.iter().filter(|d| d["role"] == "related").count())
        .unwrap_or(0);
    let graph = &data["graph"];
    let nodes = graph["nodeCount"]
        .as_u64()
        .or_else(|| graph["nodes"].as_array().map(|a| a.len() as u64))
        .unwrap_or(0);
    let edges = graph["edgeCount"]
        .as_u64()
        .or_else(|| graph["edges"].as_array().map(|a| a.len() as u64))
        .unwrap_or(0);
    let relations_list = graph["relations"].as_array();
    let relations_count = graph["relationCount"]
        .as_u64()
        .or_else(|| relations_list.map(|a| a.len() as u64))
        .unwrap_or(edges);
    let unresolved = data["unresolved"].as_array().map(Vec::len).unwrap_or(0);

    let mut text = format!(
        "## Explore ({view}) · {targets} target(s), {related} related · {relations_count} relation(s) ({nodes} nodes, {edges} edges) · unresolved: {unresolved} · freshness index={index_freshness} file={file_freshness}\n"
    );

    if let Some(body) = data["text"].as_str().filter(|s| !s.is_empty()) {
        text.push('\n');
        text.push_str(body);
        if !text.ends_with('\n') {
            text.push('\n');
        }
        return Some(text);
    }

    if let Some(defs) = definitions.filter(|d| !d.is_empty()) {
        text.push_str("\n**Definitions**\n");
        for def in defs.iter().take(8) {
            let path = def["path"].as_str().unwrap_or("");
            let role = def["role"].as_str().unwrap_or("symbol");
            let sym = &def["symbol"];
            let name = sym["name"].as_str().unwrap_or("?");
            let line = sym["range"][0]
                .as_u64()
                .or_else(|| sym["start"].as_u64())
                .unwrap_or(1);
            let sig = sym["signature"].as_str().unwrap_or("");
            if sig.is_empty() {
                text.push_str(&format!(
                    "- [{role}] `{}` ({path}:{line})\n",
                    super::clip(name, 80)
                ));
            } else {
                text.push_str(&format!(
                    "- [{role}] `{}` ({path}:{line}) — `{}`\n",
                    super::clip(name, 80),
                    super::clip(sig, 140)
                ));
            }
        }
    }

    if let Some(rels) = relations_list.filter(|r| !r.is_empty()) {
        text.push_str("\n**Top relationships**\n");
        for rel in rels.iter().take(10) {
            let src = rel["source"].as_str().unwrap_or("?");
            let dst = rel["target"].as_str().unwrap_or("?");
            let kind = rel["kind"].as_str().unwrap_or("relates");
            text.push_str(&format!(
                "- `{}` —[{kind}]→ `{}`\n",
                super::clip(src, 80),
                super::clip(dst, 80)
            ));
        }
    } else if let Some(edges_arr) = graph["edges"].as_array().filter(|e| !e.is_empty()) {
        text.push_str("\n**Top relationships**\n");
        for edge in edges_arr.iter().take(10) {
            let src = edge["source"].as_str().unwrap_or("?");
            let dst = edge["target"].as_str().unwrap_or("?");
            let kind = edge["kind"].as_str().unwrap_or("calls");
            text.push_str(&format!(
                "- `{}` —[{kind}]→ `{}`\n",
                super::clip(src, 80),
                super::clip(dst, 80)
            ));
        }
    }

    if definitions.is_some_and(|d| !d.is_empty())
        || relations_list.is_some()
        || graph.is_object()
        || data["candidates"].is_array()
    {
        Some(text)
    } else {
        None
    }
}

fn render_search_fallback(child: &Value) -> Option<String> {
    let data = &child["data"];
    let matches = data["matches"].as_array()?;
    let omitted = child["omitted"]["matches"]
        .as_u64()
        .or_else(|| data["omitted"].as_u64())
        .unwrap_or(0) as usize;
    let total = data["totalMatches"]
        .as_u64()
        .or_else(|| data["total"].as_u64())
        .or_else(|| data["count"].as_u64())
        .map(|n| n as usize)
        .unwrap_or(matches.len() + omitted);
    let mut text = format!(
        "## Search matches · {} shown ({total} total)\n\n",
        matches.len()
    );
    for item in matches.iter().take(12) {
        let file = item["file"]
            .as_str()
            .or_else(|| item["path"].as_str())
            .unwrap_or("?");
        let line = item["line"]
            .as_u64()
            .or_else(|| item["range"][0].as_u64())
            .or_else(|| item["start"].as_u64())
            .unwrap_or(1);
        let label = item["name"]
            .as_str()
            .or_else(|| item["text"].as_str())
            .or_else(|| item["signature"].as_str())
            .unwrap_or("");
        text.push_str(&format!(
            "- `{file}:{line}` {}\n",
            super::clip(label, 140)
        ));
    }
    Some(text)
}

fn render_symbols_fallback(child: &Value) -> Option<String> {
    let data = &child["data"];
    let candidates = data["candidates"].as_array()?;
    let path = data["path"]
        .as_str()
        .or_else(|| child["scope"]["path"].as_str())
        .unwrap_or("");
    let mut text = format!(
        "## Symbols {} · {} candidate(s)\n\n",
        super::clip(path, 120),
        candidates.len()
    );
    for candidate in candidates.iter().take(10) {
        let name = candidate["name"].as_str().unwrap_or("?");
        let container = candidate["container"].as_str().unwrap_or("");
        let id = candidate["symbolId"].as_str().unwrap_or("");
        if container.is_empty() {
            text.push_str(&format!(
                "- `{}` (`{}`)\n",
                super::clip(name, 80),
                super::clip(id, 120)
            ));
        } else {
            text.push_str(&format!(
                "- `{}::{}` (`{}`)\n",
                super::clip(container, 80),
                super::clip(name, 80),
                super::clip(id, 120)
            ));
        }
    }
    Some(text)
}

fn render_sql_fallback(data: &Value) -> Option<String> {
    if let Some(status) = data["status"].as_str() {
        let engine = data["engine"].as_str().unwrap_or("sql");
        return Some(format!(
            "## SQL status ({engine}) · {status} · configured={} · readOnly={}\n",
            data["configured"].as_bool().unwrap_or(false),
            data["readOnly"].as_bool().unwrap_or(true)
        ));
    }
    for key in ["items", "objects", "tables"] {
        if let Some(items) = data[key].as_array() {
            let mut text = format!("## SQL {key} · {} item(s)\n\n", items.len());
            for item in items.iter().take(16) {
                let name = item["name"]
                    .as_str()
                    .or_else(|| item["resolvedName"].as_str())
                    .or_else(|| item["table"].as_str())
                    .unwrap_or("?");
                let schema = item["schema"].as_str().unwrap_or("");
                let kind = item["kind"]
                    .as_str()
                    .or_else(|| item["objectType"].as_str())
                    .unwrap_or("");
                if schema.is_empty() {
                    text.push_str(&format!(
                        "- `{}` {}\n",
                        super::clip(name, 100),
                        super::clip(kind, 40)
                    ));
                } else {
                    text.push_str(&format!(
                        "- `{}.{}` {}\n",
                        super::clip(schema, 60),
                        super::clip(name, 100),
                        super::clip(kind, 40)
                    ));
                }
            }
            return Some(text);
        }
    }
    None
}

fn render_git_fallback(data: &Value) -> Option<String> {
    if data["changed"] == false {
        let path = data["path"].as_str().unwrap_or(".");
        let comparison = data["comparison"].as_str().unwrap_or("worktree");
        return Some(format!(
            "## Git diff {} · {} · no changes\n",
            super::clip(path, 120),
            super::clip(comparison, 40)
        ));
    }
    None
}

fn render_verify_child(data: &Value) -> Option<String> {
    let obj = data.as_object()?;
    if obj.is_empty() {
        return None;
    }
    let status = data["status"].as_str().unwrap_or("completed");
    let action = data["action"].as_str().unwrap_or("verify");
    let errors = data["errorCount"]
        .as_u64()
        .or_else(|| data["counts"]["errors"].as_u64())
        .or_else(|| data["errors"].as_array().map(|a| a.len() as u64))
        .unwrap_or(0);
    let warnings = data["warningCount"]
        .as_u64()
        .or_else(|| data["counts"]["warnings"].as_u64())
        .or_else(|| data["warnings"].as_array().map(|a| a.len() as u64))
        .unwrap_or(0);
    let mut text =
        format!("## Verify ({action}) · {status} · {errors} error(s), {warnings} warning(s)\n");
    for key in ["errors", "warnings", "items"] {
        if let Some(items) = data[key].as_array().filter(|a| !a.is_empty()) {
            text.push_str(&format!("\n**Top {key}**\n"));
            for item in items.iter().take(8) {
                if let Some(s) = item.as_str() {
                    text.push_str(&format!("- {}\n", super::clip(s, 180)));
                } else {
                    let code = item["code"].as_str().unwrap_or("");
                    let msg = item["message"].as_str().unwrap_or("");
                    let file = item["file"]
                        .as_str()
                        .or_else(|| item["path"].as_str())
                        .unwrap_or("");
                    let line = item["line"].as_u64().unwrap_or(0);
                    let loc = if file.is_empty() {
                        String::new()
                    } else if line > 0 {
                        format!("{file}:{line} ")
                    } else {
                        format!("{file} ")
                    };
                    text.push_str(&format!(
                        "- {loc}{}: {}\n",
                        super::clip(code, 40),
                        super::clip(msg, 180)
                    ));
                }
            }
        }
    }
    Some(text)
}

fn render_failed_child(child: &Value) -> Option<String> {
    let state = child["state"].as_str().unwrap_or("error");
    let diagnostics = child["diagnostics"].as_array();
    let data = &child["data"];
    let has_diagnostics = diagnostics.is_some_and(|d| !d.is_empty());
    let has_empty_data = data.as_object().is_none_or(|m| m.is_empty());
    if !has_diagnostics && has_empty_data {
        return None;
    }
    let count = diagnostics.map(Vec::len).unwrap_or(0);
    let mut text = format!("**Status:** {state} ({count} diagnostic(s))\n");
    if let Some( path ) = data["path"]
        .as_str()
        .or_else(|| child["scope"]["path"].as_str())
        .filter(|s| !s.is_empty())
    {
        text.push_str(&format!("**Target:** `{}`\n", super::clip(path, 160)));
    }
    if let Some(candidates) = data["candidates"].as_array().filter(|c| !c.is_empty()) {
        text.push_str(&format!("\n**Candidates ({})**\n", candidates.len()));
        for candidate in candidates.iter().take(8) {
            let name = candidate["name"].as_str().unwrap_or("?");
            let id = candidate["symbolId"].as_str().unwrap_or("");
            text.push_str(&format!(
                "- `{}` (`{}`)\n",
                super::clip(name, 80),
                super::clip(id, 120)
            ));
        }
    }
    Some(text)
}

#[cfg(test)]
#[path = "readable_batch_tests.rs"]
mod tests;

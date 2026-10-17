use super::{
    project::Project,
    rust_symbols::{self, Symbol},
};
use harness_domain::{
    Failure,
    contracts::{OutlineArgs, ReadSymbolArgs, SymbolView},
    execution::OperationContext,
    hash,
    navigation::anchors,
    results::utf16_len,
};
use serde_json::{Value, json};
use std::path::Path;

fn snapshot(
    project: &Project,
    path: &str,
    context: &OperationContext,
) -> Result<(String, String, Vec<Symbol>, &'static str), Failure> {
    let ext = Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let lang = match ext.as_str() {
        "rs" => "rust",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "ts" | "mts" | "cts" | "tsx" => "typescript",
        _ => {
            return Err((
                "PARSER_UNAVAILABLE",
                "Symbol navigation only supports Rust, JavaScript and TypeScript in this release",
            ));
        }
    };

    let bytes = project.bytes_with_context(path, context)?;
    let digest = hash(&bytes);
    let text = String::from_utf8(bytes).map_err(|_| ("ENCODING", "Source is not valid UTF-8"))?;

    let symbols = match lang {
        "rust" => rust_symbols::collect(&text, &digest, context)?,
        _ => super::ts_symbols::collect(path, &text, &digest, context)?,
    };
    Ok((text, digest, symbols, lang))
}

pub(super) fn outline(
    project: &Project,
    args: &OutlineArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    let max = args.max_chars.unwrap_or(8000);
    if !(1000..=16000).contains(&max)
        || args
            .query
            .as_ref()
            .is_some_and(|q| q.len() > 1000 || q.contains('\0'))
    {
        return Err((
            "ARGUMENT",
            "maxChars must be 1000..16000; query limited to 1000 bytes",
        ));
    }
    let (_, digest, all, lang) = snapshot(project, &args.path, context)?;
    let mut ranked: Vec<_> = all.iter().enumerate().collect();
    let query = args.query.as_deref().unwrap_or("").to_lowercase();
    if !query.is_empty() {
        ranked.sort_by_key(|(index, item)| {
            let score = if item.name.to_lowercase() == query {
                0
            } else if item.name.to_lowercase().contains(&query) {
                1
            } else if item.container.to_lowercase().contains(&query) {
                2
            } else if item.signature.to_lowercase().contains(&query) {
                3
            } else {
                4
            };
            (score, *index)
        });
    }
    let mut selected = Vec::new();
    let mut used = 0;
    for (_, item) in ranked {
        context.check()?;
        let cost = utf16_len(
            &serde_json::to_string(item).map_err(|_| ("INTERNAL", "Cannot serialize symbol"))?,
        );
        if used + cost <= max {
            selected.push(item);
            used += cost;
        }
    }
    let omitted = all.len() - selected.len();
    let coverage = match lang {
        "rust" => "declarations, inline modules, impl and trait members; no local items or macro expansion",
        _ => "declarations, classes, interfaces, types, enums and members; no local block items",
    };
    let mut result = json!({"path":args.path,"hash":digest,"items":selected,"omitted":omitted,
        "selection":{"method":if query.is_empty() {"source-order"} else {"lexical-ranking"},"maxChars":max,"usedChars":used,"unit":"JSON UTF-16 units, not tokens"},
        "syntaxOnly":true,"language":lang,"coverage":coverage});
    if omitted > 0 {
        result["_detail"] = json!({"path":args.path,"hash":digest,"items":all});
    }
    Ok(result)
}

pub(super) fn read_symbol(
    project: &Project,
    args: &ReadSymbolArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    if args.symbol.is_empty()
        || args.symbol.len() > 1024
        || args.container.as_ref().is_some_and(|c| c.len() > 4096)
        || args
            .symbol_id
            .as_ref()
            .is_some_and(|id| id.len() != 24 || !id.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(("ARGUMENT", "Invalid symbol selector"));
    }
    let (text, digest, all, _) = snapshot(project, &args.path, context)?;
    let matches: Vec<_> = all
        .iter()
        .filter(|s| {
            s.name == args.symbol
                && args.container.as_ref().is_none_or(|c| &s.container == c)
                && args.symbol_id.as_ref().is_none_or(|id| &s.id == id)
        })
        .collect();
    if matches.is_empty() {
        return Err(if args.symbol_id.is_some() {
            (
                "STALE_SYMBOL",
                "Symbol ID is stale or does not match; request outline again",
            )
        } else {
            ("SYMBOL", "Symbol not found; request outline")
        });
    }
    if matches.len() > 1 {
        return Ok(
            json!({"ok":false,"candidates":matches.iter().take(5).collect::<Vec<_>>(),
            "diagnostics":[{"code":"AMBIGUOUS_SYMBOL","message":"Specify an exact container or current symbolId"}],
            "_detail":{"path":args.path,"hash":digest,"candidates":matches}}),
        );
    }
    let item = matches[0];
    if matches!(args.view, Some(SymbolView::Signature)) {
        return Ok(
            json!({"path":args.path,"hash":digest,"view":"signature","symbol":item,"syntaxOnly":true,"editReady":false,
            "notice":"Folded signature is not oldText; request full with this symbolId for exact source"}),
        );
    }
    let body = &text[item.bytes.clone()];
    let large = item.end - item.start + 1 > 100 || body.len() > 16000;
    let mut result = json!({"path":args.path,"hash":digest,"symbol":item,"anchors":anchors(&text,item.start,item.end),
        "text":if large { None } else { Some(body) },"truncated":large,"syntaxOnly":true});
    if large {
        result["_detail"] = json!({"path":args.path,"hash":digest,"symbol":item,"text":body});
    }
    Ok(result)
}

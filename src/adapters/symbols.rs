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
    include_structure: bool,
) -> Result<(String, String, Vec<Symbol>, &'static str, Option<Value>), Failure> {
    let ext = Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let lang = match ext.as_str() {
        "rs" => "rust",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "ts" | "mts" | "cts" | "tsx" => "typescript",
        "vue" => "vue",
        "astro" => "astro",
        _ => {
            return Err((
                "PARSER_UNAVAILABLE",
                "Symbol navigation only supports Rust, JavaScript, TypeScript, Vue and Astro in this release",
            ));
        }
    };

    let bytes = project.bytes_with_context(path, context)?;
    let digest = hash(&bytes);
    let text = String::from_utf8(bytes).map_err(|_| ("ENCODING", "Source is not valid UTF-8"))?;

    let mut structure = None;
    let symbols = match lang {
        "rust" => rust_symbols::collect(&text, &digest, context)?,
        "vue" => {
            let document = super::vue_structure::parse(&text, &digest, context)?;
            if include_structure {
                structure = Some(document.structure(&text));
            }
            document.symbols
        }
        "astro" => super::astro_syntax::collect_symbols(path, &text, &digest, context)?,
        _ => super::ts_symbols::collect(path, &text, &digest, context)?,
    };
    Ok((text, digest, symbols, lang, structure))
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
    let (_, digest, all, lang, structure) = snapshot(project, &args.path, context, true)?;
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
    let mut by_kind = std::collections::BTreeMap::<&str, usize>::new();
    for item in &all {
        *by_kind.entry(item.kind).or_default() += 1;
    }
    let coverage = match lang {
        "rust" => {
            "declarations, inline modules, impl and trait members; no local items or macro expansion"
        }
        "vue" => {
            "native SFC/template structural parser, JS/TS AST declarations and conservative CSS/SCSS selector associations; not Vue compiler/type checking"
        }
        "astro" => {
            "frontmatter TypeScript/JavaScript declarations, Props types and imports; template markup not type-checked"
        }
        _ => "declarations, classes, interfaces, types, enums and members; no local block items",
    };
    let mut result = json!({"path":args.path,"hash":digest,"items":selected,"omitted":omitted,
        "selection":{"method":if query.is_empty() {"source-order"} else {"lexical-ranking"},"maxChars":max,"usedChars":used,"unit":"JSON UTF-16 units, not tokens"},
        "syntaxOnly":true,"language":lang,"coverage":coverage,
        "inventory":{"total":all.len(),"returned":selected.len(),"omitted":omitted,"byKind":by_kind}});
    if omitted > 0 {
        result["_detail"] = json!({"path":args.path,"hash":digest,"items":all});
    }
    if let Some(structure) = structure {
        // The full structural result is recoverable even when the inline budget is small.
        if utf16_len(&structure.to_string()) + used <= max {
            result["structure"] = structure;
        } else {
            result["structure"] =
                json!({"omitted":true,"reason":"maxChars","recoverable":"detail"});
            result["_detail"] =
                json!({"path":args.path,"hash":digest,"items":all,"structure":structure});
            result["_complete"] = json!(false);
        }
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
    let (text, digest, all, _, _) = snapshot(project, &args.path, context, false)?;
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
    // Full-symbol reads are bounded by bytes, not formatting: a modest 101-line
    // function should not require an extra detail round-trip solely for line count.
    // harness_read keeps its separate 100-line fragment limit.
    let large = body.len() > 16000;
    let mut result = json!({"path":args.path,"hash":digest,"symbol":item,"anchors":anchors(&text,item.start,item.end),
        "text":if large { None } else { Some(body) },"truncated":large,"syntaxOnly":true});
    if large {
        result["_detail"] = json!({"path":args.path,"hash":digest,"symbol":item,"text":body});
    }
    Ok(result)
}

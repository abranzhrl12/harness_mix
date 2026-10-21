use super::{
    parsers::{css::symbols as css_symbols, model::Symbol},
    project::Project,
    rust_symbols,
};
use harness_domain::{
    Failure,
    contracts::{OutlineArgs, OutlineFilterKind, OutlineView, ReadSymbolArgs, SymbolView},
    execution::OperationContext,
    hash,
    navigation::anchors,
    parsing::{SourceFormat, SourceLanguage},
    results::utf16_len,
};
use serde_json::{Value, json};
use std::path::Path;

pub(super) fn snapshot(
    project: &Project,
    path: &str,
    context: &OperationContext,
    include_structure: bool,
) -> Result<(String, String, Vec<Symbol>, &'static str, Option<Value>), Failure> {
    let language = SourceFormat::from_path(Path::new(path))
        .ok_or((
            "PARSER_UNAVAILABLE",
            "Symbol navigation supports Rust, JavaScript, TypeScript, Vue, Astro, C#, CSS and HTML",
        ))?
        .language();
    let lang = language.protocol_name();

    let bytes = project.bytes_with_context(path, context)?;
    let digest = hash(&bytes);
    let text = String::from_utf8(bytes).map_err(|_| ("ENCODING", "Source is not valid UTF-8"))?;

    let mut structure = None;
    let symbols = match language {
        SourceLanguage::Rust => rust_symbols::collect(&text, &digest, context)?,
        SourceLanguage::CSharp => super::csharp_syntax::collect(&text, &digest, context)?,
        SourceLanguage::Razor => {
            let mut items = super::csharp_syntax::collect_cshtml(&text, &digest, context)?;
            items.extend(css_symbols::collect(&text, &digest, true, context)?);
            items.sort_by_key(|item| item.bytes.start);
            items
        }
        SourceLanguage::Css | SourceLanguage::Html => {
            css_symbols::collect(&text, &digest, language == SourceLanguage::Html, context)?
        }
        SourceLanguage::Vue => {
            let document = super::vue_structure::parse(&text, &digest, context)?;
            if include_structure {
                structure = Some(document.structure(&text));
            }
            document.symbols
        }
        SourceLanguage::Astro => {
            super::astro_syntax::collect_symbols(path, &text, &digest, context)?
        }
        SourceLanguage::JavaScript | SourceLanguage::TypeScript => {
            super::ts_symbols::collect(path, &text, &digest, context)?
        }
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
    let eligible: Vec<_> = all
        .iter()
        .filter(|item| match args.filter_kind {
            Some(OutlineFilterKind::Method) => item.kind == "method",
            None => true,
        })
        .collect();
    let filtered = all.len() - eligible.len();
    let mut ranked: Vec<_> = eligible.iter().enumerate().collect();
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
        let value = match args.view {
            Some(OutlineView::Compact) => compact_symbol(item),
            None => {
                serde_json::to_value(item).map_err(|_| ("INTERNAL", "Cannot serialize symbol"))?
            }
        };
        let cost = utf16_len(&value.to_string());
        if used + cost <= max {
            selected.push(value);
            used += cost;
        }
    }
    let omitted = eligible.len() - selected.len();
    let detail_items: Vec<_> = if omitted > 0 {
        eligible
            .iter()
            .map(|item| match args.view {
                Some(OutlineView::Compact) => compact_symbol(item),
                None => serde_json::to_value(item).unwrap_or(Value::Null),
            })
            .collect()
    } else {
        Vec::new()
    };
    let mut by_kind = std::collections::BTreeMap::<&str, usize>::new();
    for item in &all {
        *by_kind.entry(item.kind).or_default() += 1;
    }
    let coverage = match lang {
        "css" => {
            "conservative CSS rule scanner; rules and grouping at-rule containers; no CSS validation or nested selector expansion"
        }
        "html" => "CSS rules in embedded style blocks; no HTML or CSS validation",
        "razor" => {
            "C# declarations and CSS rules in embedded style blocks; no Razor compilation or CSS validation"
        }
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
    let returned = selected.len();
    let mut result = json!({"path":args.path,"hash":digest,"items":selected,"omitted":omitted,"filtered":filtered,
        "selection":{"method":if query.is_empty() {"source-order"} else {"lexical-ranking"},"maxChars":max,"usedChars":used,"unit":"JSON UTF-16 units, not tokens"},
        "syntaxOnly":true,"language":lang,"coverage":coverage,
        "inventory":{"total":all.len(),"returned":returned,"omitted":omitted,"filtered":filtered,"byKind":by_kind}});
    if omitted > 0 {
        result["_detail"] = json!({"path":args.path,"hash":digest,"items":&detail_items});
    }
    if let Some(structure) = structure {
        // The full structural result is recoverable even when the inline budget is small.
        if utf16_len(&structure.to_string()) + used <= max {
            result["structure"] = structure;
        } else {
            result["structure"] =
                json!({"omitted":true,"reason":"maxChars","recoverable":"detail"});
            let structural_items: Vec<_> = if omitted > 0 {
                detail_items
            } else {
                eligible
                    .iter()
                    .map(|item| match args.view {
                        Some(OutlineView::Compact) => compact_symbol(item),
                        None => serde_json::to_value(item).unwrap_or(Value::Null),
                    })
                    .collect()
            };
            result["_detail"] = json!({"path":args.path,"hash":digest,"items":structural_items,"structure":structure});
            result["_complete"] = json!(false);
        }
    }
    Ok(result)
}

fn compact_symbol(item: &Symbol) -> Value {
    json!({
        "name": item.name,
        "signature": item.signature,
        "attributes": item.attributes,
        "container": item.container,
        "line": item.start,
        "symbolId": item.id,
    })
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

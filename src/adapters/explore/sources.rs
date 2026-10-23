use super::{model::Node, presentation};
use crate::adapters::project::Project;
use harness_domain::{
    Failure,
    contracts::{ExploreArgs, ReadSymbolArgs, SymbolView},
    execution::OperationContext,
    results::{prefix, utf16_len},
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct LoadedSources {
    pub(super) definitions: Vec<Value>,
    pub(super) unresolved: Vec<Value>,
    pub(super) text: String,
    pub(super) context_omitted: usize,
    pub(super) truncated: bool,
    pub(super) definitions_limited: bool,
}

pub(super) fn load(
    project: &Project,
    args: &ExploreArgs,
    context: &OperationContext,
    targets: &[Node],
    nodes: &BTreeMap<String, Node>,
    edges: &[Value],
    max_files: usize,
    max_chars: usize,
    summary: bool,
    symbol: &str,
) -> Result<LoadedSources, Failure> {
    let mut definitions = Vec::new();
    let mut delivered_symbols = BTreeSet::new();
    let mut unresolved = Vec::new();
    for name in super::query_symbols(args) {
        if !targets.iter().any(|n| n.name == name) {
            unresolved.push(json!({"symbol":name,"role":"target","code":"TARGET_NOT_RESOLVED","message":"No selected indexed target; not proof of global absence"}));
        }
    }
    let mut text = if summary {
        format!("Símbolo: {symbol}\nRelaciones indexadas; vigencia no certificada.\n")
    } else {
        presentation::header(symbol)
    };
    let (related_context, context_omitted) = if summary {
        (Vec::new(), 0)
    } else {
        presentation::context_nodes(targets, nodes, edges, max_files)
    };
    let related_ids: BTreeSet<_> = related_context.iter().map(|n| n.id.as_str()).collect();
    let context_targets: Vec<_> = targets
        .iter()
        .chain(related_context.iter().copied())
        .collect();
    let mut truncated = false;
    let mut definitions_limited = false;
    for selected in &context_targets {
        if selected.kind == "file" && super::razor_file_selector(args) {
            continue;
        }
        let file = &selected.file_path;
        let related = related_ids.contains(selected.id.as_str());
        context.check()?;
        let read = if selected.id.starts_with("sql-local:")
            && matches!(selected.kind.as_str(), "stored_procedure" | "table")
        {
            super::super::index_store::sql_objects::read_object(
                project,
                file,
                &selected.id,
                context,
            )
        } else {
            super::super::symbols::read_symbol(
                project,
                &ReadSymbolArgs {
                    external_scope: None,
                    path: file.clone(),
                    symbol: selected.name.clone(),
                    container: if related {
                        None
                    } else {
                        args.container.clone()
                    },
                    symbol_id: None,
                    view: if summary {
                        Some(SymbolView::Signature)
                    } else {
                        None
                    },
                    session: None,
                },
                context,
            )
        };
        match read {
            Ok(value) => {
                let identity = (
                    file.clone(),
                    value["hash"].to_string(),
                    value["symbol"]["symbolId"].to_string(),
                );
                if !delivered_symbols.insert(identity) {
                    continue;
                }
                let body = if summary {
                    value["symbol"]["signature"].as_str().unwrap_or("")
                } else {
                    value["text"]
                        .as_str()
                        .or_else(|| value["_detail"]["text"].as_str())
                        .unwrap_or("")
                };
                let remaining = 64000usize.saturating_sub(utf16_len(&text));
                let body_budget = if summary {
                    512
                } else if related {
                    2000
                } else {
                    (max_chars / targets.len().max(1)).max(1000).min(16000)
                };
                let preview = prefix(body, remaining.min(body_budget));
                truncated |= utf16_len(body) > utf16_len(preview);
                definitions_limited |= utf16_len(body) > utf16_len(preview);
                if summary {
                    text.push_str(&format!("\n### {file}\n{preview}\n"));
                } else {
                    text.push_str(&presentation::source(
                        file,
                        value["symbol"]["start"].as_u64().unwrap_or(1),
                        preview,
                        utf16_len(body) > utf16_len(preview),
                    ));
                }
                definitions.push(json!({"path":file,"hash":value["hash"],"symbol":value["symbol"],"role":if related {"related"} else {"target"},
                    "_bodyComplete":!summary && utf16_len(body) == utf16_len(preview),"_textEnd":utf16_len(&text)}));
            }
            Err((code @ ("CANCELLED" | "TIMEOUT"), message)) => return Err((code, message)),
            Err((code, _)) => unresolved.push(json!({"path":file,"symbol":selected.name,"role":if related {"related"} else {"target"},"code":code,"message":"Indexed candidate not resolved by the current local parser; use outline/read_symbol with an exact selector"})),
        }
    }
    if !summary && context_omitted > 0 {
        text.push_str(&format!("\n> {context_omitted} related symbols omitted by the 4-symbol/file context budget; select a name from graph and call read_symbol.\n"));
    }
    Ok(LoadedSources {
        definitions,
        unresolved,
        text,
        context_omitted,
        truncated,
        definitions_limited,
    })
}

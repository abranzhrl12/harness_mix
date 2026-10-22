use harness_domain::{Failure, execution::OperationContext};
use serde_json::Value;

pub fn language(path: &str) -> Option<&'static str> {
    match path.rsplit_once('.')?.1.to_ascii_lowercase().as_str() {
        "rs" => Some("rust"),
        "cs" => Some("csharp"),
        "cshtml" => Some("razor"),
        "vue" => Some("vue"),
        "html" | "htm" => Some("html"),
        "ts" | "mts" | "cts" => Some("typescript"),
        "tsx" => Some("tsx"),
        "js" | "mjs" | "cjs" => Some("javascript"),
        "jsx" => Some("jsx"),
        "scss" => Some("scss"),
        _ => None,
    }
}

pub fn extract(path: &str, source: &str, context: &OperationContext) -> Result<Value, Failure> {
    context.check()?;
    if source.len() > super::super::index_sync::MAX_SOURCE_BYTES {
        return Err((
            "EXTRACTION_LIMIT",
            "Source exceeds native extraction size budget",
        ));
    }
    let lang = language(path).ok_or((
        "SYNC_LANGUAGE",
        "Source language has no native CodeGraph extractor",
    ))?;
    if lang == "razor" {
        return super::razor::extract_razor(path, source, context);
    }
    if lang == "vue" {
        return super::vue::extract(path, source, context);
    }
    if lang == "html" {
        return super::angular::html(path, source, context);
    }
    if lang == "scss" {
        return super::scss::extract(path, source, context);
    }
    let prepared = if lang == "csharp" {
        super::csharp::blank_csharp(source)?
    } else {
        source.to_owned()
    };
    let raw = codegraph_native::extract(path, &prepared, lang).map_err(|_| {
        (
            "EXTRACTION_DEFERRED",
            "Native extractor deferred this source; no Node fallback was executed",
        )
    })?;
    context.check()?;
    let mut graph = super::decode::decode(&raw, path, lang, context)?;
    if matches!(lang, "typescript" | "tsx" | "javascript" | "jsx") {
        super::angular::enrich(path, source, &mut graph, context)?;
    }
    Ok(graph)
}

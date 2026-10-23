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
    super::take_diagnostic();
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
    let raw = codegraph_native::extract(path, &prepared, lang).map_err(|error| {
        let mut diagnostic = serde_json::json!({"file":path,"language":lang,"stage":"native_extract","message":error});
        if lang == "rust" {
            let parsed = ra_ap_syntax::SourceFile::parse(source, ra_ap_syntax::Edition::Edition2024);
            let errors: Vec<_> = parsed.errors().iter().map(|error| {
                let offset = usize::from(error.range().start());
                let prefix = &source[..offset.min(source.len())];
                let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
                let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
                serde_json::json!({"line":line,"column":column,"message":error.to_string()})
            }).collect();
            diagnostic["errors"] = serde_json::json!(errors);
        }
        super::set_diagnostic(diagnostic);
        (
            "EXTRACTION_DEFERRED",
            "Native extractor deferred this source; no Node fallback was executed",
        )
    })?;
    context.check()?;
    let mut graph = super::decode::decode(&raw, path, lang, context)?;
    if matches!(lang, "typescript" | "tsx" | "javascript" | "jsx") {
        super::angular::enrich(path, source, &mut graph, context)?;
        super::vue_router::enrich(path, source, &mut graph, context)?;
    }
    Ok(graph)
}

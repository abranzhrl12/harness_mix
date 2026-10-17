//! Decode the pinned CodeGraph kernel ABI without JavaScript or N-API.
use codegraph_native::buffers::*;
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};

const CONTRACT: Failure = (
    "EXTRACTION_CONTRACT",
    "Invalid native CodeGraph extraction buffers",
);

pub(super) fn preview(
    project: &super::project::Project,
    plan: &super::index_sync::Plan,
    observations: &std::collections::BTreeMap<String, super::index_sync::Observation>,
    context: &OperationContext,
) -> Result<Value, Failure> {
    use std::time::{Duration, Instant};
    let started = Instant::now();
    let candidates = plan
        .added
        .iter()
        .chain(&plan.modified)
        .chain(&plan.retry_extraction)
        .filter(|path| language(path).is_some())
        .collect::<Vec<_>>();
    let mut files = Vec::new();
    for path in candidates.iter().take(4) {
        context.check()?;
        if started.elapsed() >= Duration::from_secs(5) {
            break;
        }
        let run = || {
            let validated = project.target(path)?;
            #[cfg(windows)]
            let _pins = super::edit_commit::pin_directory(
                project,
                validated
                    .parent()
                    .ok_or(("PATH", "Missing source parent"))?,
            )?;
            #[cfg(not(windows))]
            let _ = validated;
            let bytes = project.bytes_with_limit(
                path,
                context,
                super::index_sync::MAX_SOURCE_BYTES as u64,
            )?;
            let digest = super::index_sync::source_hash(&bytes)?;
            if !matches!(observations.get(*path), Some(super::index_sync::Observation::Content(observed)) if *observed == digest)
            {
                return Err(("STALE", "Source changed after inventory"));
            }
            let source =
                std::str::from_utf8(&bytes).map_err(|_| ("ENCODING", "Source is not UTF-8"))?;
            let extracted = extract(path, source, context)?;
            Ok(
                json!({"path":path,"sourceHash":digest,"status":"extracted_not_published",
                "nodes":extracted["nodes"].as_array().ok_or(CONTRACT)?.len(),
                "edges":extracted["edges"].as_array().ok_or(CONTRACT)?.len(),
                "unresolvedReferences":extracted["unresolvedReferences"].as_array().ok_or(CONTRACT)?.len()}),
            )
        };
        match run() {
            Ok(value) => files.push(value),
            Err(failure @ ("CANCELLED" | "TIMEOUT", _)) => return Err(failure),
            Err((code, _)) => files.push(json!({"path":path,"status":"deferred","code":code})),
        }
    }
    Ok(
        json!({"provider":"codegraph-v1.6.2-native-subset","files":files,
        "pendingFiles":candidates.len().saturating_sub(files.len()),"maxFiles":4,
        "relationsResolved":false,"published":false,"sourceSnapshotAtomic":false}),
    )
}

pub fn language(path: &str) -> Option<&'static str> {
    match path.rsplit_once('.')?.1 {
        "rs" => Some("rust"),
        "cs" => Some("csharp"),
        "ts" | "mts" | "cts" => Some("typescript"),
        "tsx" => Some("tsx"),
        "js" | "mjs" | "cjs" => Some("javascript"),
        "jsx" => Some("jsx"),
        _ => None,
    }
}

pub fn extract(path: &str, source: &str, context: &OperationContext) -> Result<Value, Failure> {
    context.check()?;
    if source.len() > super::index_sync::MAX_SOURCE_BYTES {
        return Err((
            "EXTRACTION_LIMIT",
            "Source exceeds native extraction size budget",
        ));
    }
    let lang = language(path).ok_or((
        "SYNC_LANGUAGE",
        "Source language has no native CodeGraph extractor",
    ))?;
    let prepared = if lang == "csharp" {
        blank_csharp(source)?
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
    decode(&raw, path, lang, context)
}

// Port of languages/csharp.ts blankCsharpPreprocessorDirectives. JS replaces
// each UTF-16 unit (including CR) with one space; both conditional branches stay.
fn blank_csharp(source: &str) -> Result<String, Failure> {
    if source.contains(['\u{2028}', '\u{2029}']) || source.replace("\r\n", "").contains('\r') {
        return Err((
            "EXTRACTION_DEFERRED",
            "Unported C# preprocessor line separator",
        ));
    }
    let mut result = String::with_capacity(source.len());
    for line in source.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let tail = body.trim_start_matches([' ', '\t']);
        let directive = tail
            .strip_prefix('#')
            .map(|s| s.trim_start_matches([' ', '\t']));
        let conditional = directive.is_some_and(|s| {
            ["if", "elif", "else", "endif"].iter().any(|word| {
                s.strip_prefix(word).is_some_and(|rest| {
                    !rest
                        .as_bytes()
                        .first()
                        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
                })
            })
        });
        if conditional {
            let indent = body.len() - tail.len();
            result.push_str(&body[..indent]);
            result.push_str(&" ".repeat(tail.encode_utf16().count()));
        } else {
            result.push_str(body);
        }
        if line.ends_with('\n') {
            result.push('\n');
        }
    }
    Ok(result)
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, Failure> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .ok_or(CONTRACT)?
            .try_into()
            .map_err(|_| CONTRACT)?,
    ))
}
fn text(arena: &[u8], row: &[u8], at: usize) -> Result<Option<String>, Failure> {
    let start = u32_at(row, at)?;
    if start == NONE {
        return Ok(None);
    }
    let len = u32_at(row, at + 4)? as usize;
    let start = start as usize;
    let end = start.checked_add(len).ok_or(CONTRACT)?;
    Ok(Some(
        std::str::from_utf8(arena.get(start..end).ok_or(CONTRACT)?)
            .map_err(|_| CONTRACT)?
            .to_owned(),
    ))
}
fn required(arena: &[u8], row: &[u8], at: usize) -> Result<String, Failure> {
    text(arena, row, at)?
        .filter(|s| !s.is_empty())
        .ok_or(CONTRACT)
}
fn optional_text(
    value: &mut Value,
    key: &str,
    arena: &[u8],
    row: &[u8],
    at: usize,
) -> Result<(), Failure> {
    if let Some(s) = text(arena, row, at)? {
        value[key] = json!(s);
    }
    Ok(())
}
fn list(value: &mut Value, key: &str, arena: &[u8], row: &[u8], at: usize) -> Result<(), Failure> {
    if let Some(s) = text(arena, row, at)? {
        value[key] = json!(s.split('\0').collect::<Vec<_>>());
    }
    Ok(())
}
fn endpoint(
    ids: &[String],
    arena: &[u8],
    row: &[u8],
    index: usize,
    fallback: usize,
) -> Result<String, Failure> {
    let idx = u32_at(row, index)?;
    if idx == NONE {
        required(arena, row, fallback)
    } else {
        ids.get(idx as usize).cloned().ok_or(CONTRACT)
    }
}
fn decode(
    raw: &EmitOut,
    path: &str,
    lang: &str,
    context: &OperationContext,
) -> Result<Value, Failure> {
    if raw.meta.len() != META_SIZE
        || raw.meta[0] != KERNEL_ABI_VERSION
        || raw.arena.len() > 16 * 1024 * 1024
    {
        return Err(CONTRACT);
    }
    let counts = [
        u32_at(&raw.meta, 4)? as usize,
        u32_at(&raw.meta, 8)? as usize,
        u32_at(&raw.meta, 12)? as usize,
    ];
    for ((count, size), bytes) in counts
        .into_iter()
        .zip([NODE_ROW_SIZE, EDGE_ROW_SIZE, REF_ROW_SIZE])
        .zip([&raw.nodes, &raw.edges, &raw.refs])
    {
        if count > 20_000 || count.checked_mul(size) != Some(bytes.len()) {
            return Err(CONTRACT);
        }
    }
    if u32_at(&raw.meta, 16)? as usize != raw.arena.len() {
        return Err(CONTRACT);
    }
    let mut nodes = Vec::new();
    let mut ids = Vec::new();
    for row in raw.nodes.as_chunks::<NODE_ROW_SIZE>().0 {
        context.check()?;
        let id = required(&raw.arena, row, 36)?;
        let kind = NODE_KINDS.get(row[0] as usize).ok_or(CONTRACT)?;
        let mut node = json!({"id":id,"kind":kind,"name":required(&raw.arena,row,20)?,"qualifiedName":required(&raw.arena,row,28)?,"filePath":path,"language":lang,
            "startLine":u32_at(row,4)?,"endLine":u32_at(row,8)?,"startColumn":u32_at(row,12)?,"endColumn":u32_at(row,16)?});
        for (key, at) in [("docstring", 44), ("signature", 52), ("returnType", 76)] {
            optional_text(&mut node, key, &raw.arena, row, at)?;
        }
        list(&mut node, "decorators", &raw.arena, row, 60)?;
        list(&mut node, "typeParameters", &raw.arena, row, 68)?;
        let visibility = [
            None,
            Some("public"),
            Some("private"),
            Some("protected"),
            Some("internal"),
        ]
        .get(row[1] as usize)
        .ok_or(CONTRACT)?;
        if let Some(visibility) = visibility {
            node["visibility"] = json!(visibility);
        }
        let flags = u16::from_le_bytes([row[2], row[3]]);
        for (i, key) in ["isExported", "isAsync", "isStatic", "isAbstract"]
            .iter()
            .enumerate()
        {
            if flags & (1 << (i * 2)) != 0 {
                node[*key] = json!(flags & (1 << (i * 2 + 1)) != 0);
            }
        }
        if let Some(extra) = text(&raw.arena, row, 84)? {
            let extra: Value = serde_json::from_str(&extra).map_err(|_| CONTRACT)?;
            node.as_object_mut()
                .ok_or(CONTRACT)?
                .extend(extra.as_object().ok_or(CONTRACT)?.clone());
        }
        ids.push(id);
        nodes.push(node);
    }
    let mut edges = Vec::new();
    for row in raw.edges.as_chunks::<EDGE_ROW_SIZE>().0 {
        context.check()?;
        let mut edge = json!({"source":endpoint(&ids,&raw.arena,row,0,28)?,"target":endpoint(&ids,&raw.arena,row,4,36)?,"kind":EDGE_KINDS.get(row[8] as usize).ok_or(CONTRACT)?});
        for (key, at) in [("line", 12), ("column", 16)] {
            let n = u32_at(row, at)?;
            if n != NONE {
                edge[key] = json!(n);
            }
        }
        if let Some(p) = [None, Some("tree-sitter"), Some("scip"), Some("heuristic")]
            .get(row[9] as usize)
            .ok_or(CONTRACT)?
        {
            edge["provenance"] = json!(p);
        }
        if let Some(metadata) = text(&raw.arena, row, 20)? {
            edge["metadata"] = serde_json::from_str(&metadata).map_err(|_| CONTRACT)?;
        }
        edges.push(edge);
    }
    let mut refs = Vec::new();
    for row in raw.refs.as_chunks::<REF_ROW_SIZE>().0 {
        context.check()?;
        let kind = if row[4] == FUNCTION_REF_CODE {
            "function_ref"
        } else {
            EDGE_KINDS.get(row[4] as usize).ok_or(CONTRACT)?
        };
        let mut reference = json!({"fromNodeId":endpoint(&ids,&raw.arena,row,0,32)?,"referenceName":required(&raw.arena,row,16)?,"referenceKind":kind,"line":u32_at(row,8)?,"column":u32_at(row,12)?});
        if row[5] & REF_FLAG_FILE_PATH != 0 {
            reference["filePath"] = json!(path);
        }
        list(&mut reference, "candidates", &raw.arena, row, 24)?;
        refs.push(reference);
    }
    let errors: Value = match text(&raw.arena, &raw.meta, 20)? {
        Some(s) => serde_json::from_str(&s).map_err(|_| CONTRACT)?,
        None => json!([]),
    };
    if !errors.as_array().ok_or(CONTRACT)?.is_empty() {
        return Err((
            "EXTRACTION_DEFERRED",
            "Native extraction reported errors; index unchanged",
        ));
    }
    Ok(json!({"nodes":nodes,"edges":edges,"unresolvedReferences":refs,"errors":errors}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preprocess_preserves_utf16_columns_crlf_and_both_branches() {
        let input =
            "  #if FLAG // 😀\r\nclass A {}\r\n#else\r\nclass B {}\r\n#endif\r\n#region hi\r\n";
        let output = blank_csharp(input).unwrap();
        assert_eq!(input.encode_utf16().count(), output.encode_utf16().count());
        assert!(output.contains("class A {}\r\n"));
        assert!(output.contains("class B {}\r\n"));
        assert!(!output.contains("#if"));
        assert!(output.contains("#region hi"));
        assert_eq!(blank_csharp("#iffy\n").unwrap(), "#iffy\n");
    }
    #[test]
    fn malformed_buffers_and_cancellation_fail_closed() {
        let raw = EmitOut {
            meta: vec![],
            nodes: vec![],
            edges: vec![],
            refs: vec![],
            arena: vec![],
        };
        assert!(decode(&raw, "x.rs", "rust", &OperationContext::default()).is_err());
        let context = OperationContext::default();
        context.cancel();
        assert_eq!(
            extract("x.rs", "fn x() {}", &context).unwrap_err().0,
            "CANCELLED"
        );
    }
}

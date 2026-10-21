use super::super::ignore as ignore_policy;
use super::scan::ScanOutcome;
use harness_domain::{
    Failure,
    contracts::{Kind, SearchArgs},
};
use serde_json::{Value, json};

pub(super) fn present(args: &SearchArgs, outcome: ScanOutcome<'_>) -> Result<Value, Failure> {
    let ScanOutcome {
        file_size,
        syntax,
        context_lines,
        min_lines,
        explicit_scope,
        all,
        compact,
        visited,
        bytes,
        reserved_bytes,
        skipped,
        skipped_files,
        binary_skipped,
        parse_errors,
        stable_files,
        bounded,
        timed_out,
        cancelled,
        truncated,
        graph_outcome,
        live_languages,
        index_diagnostics,
        complete,
        execution_ms,
        folders,
        omitted_folders,
        walk_and_other_micros,
        custom_patterns,
        ..
    } = outcome;
    let key = if file_size {
        "files"
    } else if matches!(args.kind, Kind::Files) {
        "paths"
    } else {
        "matches"
    };
    let mut result = json!({"omitted":all.len().saturating_sub(compact.len()),"truncated":truncated || !complete,"_complete":complete,
        "cancelled":cancelled,"timedOut":timed_out,"complete":complete,"scanned":visited,"bytesRead":bytes,"skipped":skipped,"skippedFiles":skipped_files,"unvisitedFiles":bounded || timed_out || cancelled,"limited":bounded,
        "timings":{"executionMs":execution_ms,"directoryProcessing":folders,
            "omittedFolders":omitted_folders,
            "walkAndOtherMicros":walk_and_other_micros,
            "coverage":"top 8 folder processing times, grouped to depth 2; traversal and other work reported separately"},
        "exclusions":[".git",".hg",".svn","node_modules","target","credential and secret paths"],
        "exclusionPolicy":{
            "defaults":ignore_policy::DEFAULT_EXCLUSIONS,
            "hardExclusions":ignore_policy::HARD_EXCLUSIONS,
            "customFile":if !explicit_scope {json!(ignore_policy::FILE)} else {Value::Null},
            "customPatterns":custom_patterns.map_or(Value::Null, |count| json!(count)),
            "scopeOverride":explicit_scope,
            "gitignore":true,
            "effect":"Policy exclusions reduce scanned paths; they are not skipped-file errors and never make complete=false"
        },
        "warnings":if explicit_scope {json!([{"code":"SCOPE_OVERRIDES_EXCLUSIONS","message":"An explicit scope bypasses project .harnessignore patterns; VCS metadata, secrets and defaults remain excluded"}])} else {json!([])},
        "_detail":{},"diagnostics":[]});
    result[key] = json!(compact);
    result["_detail"][key] = json!(all);
    if args.context_lines.is_some() {
        result["contextLines"] = json!(context_lines);
        result["limits"] = json!({"contextLines":20,"metadataBytes":4*1024*1024});
    }
    if let Some(patterns) = &args.globs {
        result["globs"] = json!(patterns);
    }
    if syntax {
        let coverage = "Rust declarations and Razor directives/C# members; no local items, macro expansion, reference resolution, Razor compilation or AST patterns";
        result["kind"] = json!("syntax");
        result["syntaxOnly"] = json!(true);
        result["selector"] = json!(args.query);
        match &graph_outcome {
            Some(outcome) => {
                result["discovery"] = json!("index");
                result["language"] = json!(if outcome.languages.len() == 1 {
                    outcome.languages[0].clone()
                } else {
                    "multiple".to_owned()
                });
                result["languages"] = json!(outcome.languages);
                result["indexFreshness"] = outcome.freshness.clone();
                result["certifiedFiles"] = json!(outcome.certified_files);
                result["repairedFiles"] = json!(outcome.repaired_files);
                result["missingFiles"] = json!(outcome.missing_files);
                result["parsedFiles"] = json!(outcome.certified_files + outcome.repaired_files);
                result["parseErrors"] = json!(0);
                result["absenceProven"] = json!(false);
                result["coverage"] = json!(
                    "Indexed declarations certified against the working tree; stale files are reparsed live. Absence is not proven and the index may not cover every file."
                );
            }
            None => {
                result["discovery"] = json!("live");
                result["language"] = json!(match live_languages.len() {
                    0 => "rust",
                    1 => live_languages.first().expect("one language"),
                    _ => "multiple",
                });
                result["languages"] = json!(live_languages);
                result["coverage"] = json!(coverage);
                result["parsedFiles"] = json!(stable_files);
                result["parseErrors"] = json!(parse_errors);
            }
        }
        result["limits"] = json!({"files":10000,"fileBytes":2*1024*1024,"totalBytes":64*1024*1024,"metadataBytes":4*1024*1024,"scanMs":5000});
        result["_detail"]["coverage"] = json!(result["coverage"].clone());
        result["_detail"]["syntaxOnly"] = json!(true);
        result["_detail"]["language"] = json!(result["language"].clone());
        result["_detail"]["discovery"] = json!(result["discovery"].clone());
    }
    if file_size {
        result["kind"] = json!("file_size");
        result["minLines"] = json!(min_lines);
        result["comparison"] = json!(">");
        result["visited"] = json!(visited);
        result["scanned"] = json!(stable_files);
        result["binarySkipped"] = json!(binary_skipped);
        result["counting"] = json!(
            "LF/CRLF totals including comments/tests; empty=0; trailing LF adds no extra line"
        );
        result["limits"] = json!({"files":10000,"fileBytes":2*1024*1024,"totalBytes":64*1024*1024,"metadataBytes":4*1024*1024,"scanMs":5000});
        result["binaryPolicy"] =
            json!("Skip NUL and invalid UTF-8 files without marking those skips incomplete");
        result["reservedBytes"] = json!(reserved_bytes);
    }
    if !complete {
        result["diagnostics"] = json!([{"code":"SEARCH_INCOMPLETE","message":"búsqueda parcial por presupuesto, plazo o archivos no accesibles; revisar complete/skipped/limited"}]);
    }
    if !index_diagnostics.is_empty() {
        let mut diagnostics = result["diagnostics"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        diagnostics.extend(index_diagnostics);
        result["diagnostics"] = json!(diagnostics);
    }
    Ok(result)
}

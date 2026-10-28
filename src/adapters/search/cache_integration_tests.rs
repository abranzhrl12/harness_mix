use super::*;
use serde_json::json;
use std::fs;

#[test]
fn repeated_search_reads_changed_files_and_current_context_hash() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.rs");
    fs::write(&file, "cache_needle\n").unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args: SearchArgs = serde_json::from_value(json!({
        "query":"cache_needle", "contextLines":0
    }))
    .unwrap();
    assert_eq!(search(&project, &args).unwrap()["matches"][0]["line"], 1);
    let changed = "changed\ncache_needle\n";
    fs::write(&file, changed).unwrap();
    let result = search(&project, &args).unwrap();
    assert_eq!(result["matches"][0]["line"], 2);
    assert_eq!(result["matches"][0]["context"]["text"], "cache_needle\n");
    assert_eq!(
        result["matches"][0]["hash"],
        harness_domain::hash(changed.as_bytes())
    );
    fs::write(&file, "no longer matches\n").unwrap();
    assert_eq!(search(&project, &args).unwrap()["matches"], json!([]));
}

#[test]
fn warm_fallback_keeps_scope_unicode_and_exact_priority() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/a.rs"), "fn caféHandler() {}\n").unwrap();
    fs::write(dir.path().join("outside.rs"), "café handler\n").unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args: SearchArgs = serde_json::from_value(json!({
        "query":"café handler", "scope":"src"
    }))
    .unwrap();
    for _ in 0..2 {
        let result = search(&project, &args).unwrap();
        assert_eq!(result["retrieval"]["mode"], "terms");
        assert_eq!(result["matches"].as_array().unwrap().len(), 1);
        assert_eq!(
            result["matches"][0]["file"]
                .as_str()
                .unwrap()
                .replace('\\', "/"),
            "src/a.rs"
        );
    }
    fs::write(dir.path().join("src/b.rs"), "café handler\n").unwrap();
    let exact = search(&project, &args).unwrap();
    assert!(exact.get("retrieval").is_none());
    assert_eq!(exact["matches"].as_array().unwrap().len(), 1);
    assert_eq!(
        exact["matches"][0]["file"]
            .as_str()
            .unwrap()
            .replace('\\', "/"),
        "src/b.rs"
    );
}

#[test]
fn fallback_plan_uses_supplied_matcher_without_compiling_discarded_query() {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::new(dir.path()).unwrap();
    // An invalid exact regex proves that fallback construction never compiles it.
    // The public workflow only calls this helper after fallback eligibility checks.
    let args: SearchArgs = serde_json::from_value(json!({
        "query":"(", "regex":true
    }))
    .unwrap();
    assert!(plan::build(&project, &args).is_err());
    let matcher = grep_regex::RegexMatcher::new("fallback").unwrap();
    let fallback = plan::build_fallback(&project, &args, matcher).unwrap();
    assert!(fallback.term_retrieval);
    assert_eq!(fallback.context_lines, 2);
    let mut matched = 0;
    grep_searcher::SearcherBuilder::new()
        .build()
        .search_slice(
            fallback.matcher.as_ref().unwrap(),
            b"fallback\n",
            grep_searcher::sinks::UTF8(|_, _| {
                matched += 1;
                Ok(true)
            }),
        )
        .unwrap();
    assert_eq!(matched, 1);
}

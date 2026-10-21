use super::*;
use harness_domain::contracts::Kind;
use serde_json::json;
use std::fs;

#[test]
fn configured_ignore_filename_is_reported_even_without_a_policy_file() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("visible.txt"), "needle").unwrap();
    let project = Project::new(root.path()).unwrap();
    let mut args: SearchArgs = serde_json::from_value(json!({
        "kind":"text", "query":"needle"
    }))
    .unwrap();
    let result = search(&project, &args).unwrap();
    assert_eq!(result["exclusionPolicy"]["customFile"], ".harnessignore");
    assert!(result["exclusionPolicy"]["customPatterns"].is_null());
    args.scope = Some("visible.txt".into());
    let scoped = search(&project, &args).unwrap();
    assert!(scoped["exclusionPolicy"]["customFile"].is_null());
    assert!(scoped["exclusionPolicy"]["customPatterns"].is_null());
}

#[test]
fn context_preserves_exact_source_and_hash_at_file_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let source = "needle first  \r\n  middle 😀\r\nneedle last";
    fs::write(dir.path().join("view.cshtml"), source).unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args: SearchArgs = serde_json::from_value(json!({
        "kind":"text", "query":"needle", "contextLines":1, "limit":1
    }))
    .unwrap();
    let result = search(&project, &args).unwrap();
    let first = &result["matches"][0];
    assert_eq!(first["hash"], harness_domain::hash(source.as_bytes()));
    assert_eq!(
        first["context"],
        json!({"start":1,"end":2,
            "text":"needle first  \r\n  middle 😀\r\n"})
    );
    assert_eq!(
        result["_detail"]["matches"][1]["context"],
        json!({"start":2,"end":3,
            "text":"  middle 😀\r\nneedle last"})
    );
    assert_eq!(result["omitted"], 1);
    assert_eq!(result["complete"], true);
}

#[test]
fn context_is_opt_in_and_validated_before_search() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), "needle\n").unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args: SearchArgs = serde_json::from_value(json!({"kind":"text","query":"needle"})).unwrap();
    let result = search(&project, &args).unwrap();
    assert!(result["matches"][0].get("context").is_none());
    assert!(result["matches"][0].get("hash").is_none());
    for value in [
        json!({"kind":"text","query":"needle","contextLines":21}),
        json!({"kind":"files","query":"needle","contextLines":0}),
    ] {
        let args: SearchArgs = serde_json::from_value(value).unwrap();
        assert_eq!(search(&project, &args).unwrap_err().0, "ARGUMENT");
    }
    let args: SearchArgs =
        serde_json::from_value(json!({"kind":"text","query":"needle","contextLines":0})).unwrap();
    assert_eq!(
        search(&project, &args).unwrap()["matches"][0]["context"]["text"],
        "needle\n"
    );
}

#[test]
fn context_storage_budget_reports_partial_without_cutting_source() {
    let dir = tempfile::tempdir().unwrap();
    let line = format!("needle {}\n", "x".repeat(4096));
    fs::write(dir.path().join("a.txt"), line.repeat(100)).unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args: SearchArgs = serde_json::from_value(json!({
        "kind":"text","query":"needle","contextLines":20
    }))
    .unwrap();
    let result = search(&project, &args).unwrap();
    assert_eq!(result["limited"], true);
    assert_eq!(result["complete"], false);
    assert_eq!(result["diagnostics"][0]["code"], "SEARCH_INCOMPLETE");
    assert!(result["_detail"]["matches"].as_array().unwrap().len() < 100);
    assert_eq!(result["matches"][0]["context"]["text"], line.repeat(21));
}
#[test]
fn honors_ignore_secrets_and_compacts_without_losing_detail() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(".gitignore"), "ignored.txt\n").unwrap();
    for name in ["a.txt", "b.txt", "ignored.txt", "secret.txt", ".env"] {
        fs::write(dir.path().join(name), "needle 😀\n").unwrap();
    }
    let p = Project::new(dir.path()).unwrap();
    let args = SearchArgs {
        external_scope: None,
        context_lines: None,
        min_lines: None,
        globs: None,
        kind: Kind::Text,
        query: "needle".into(),
        scope: None,
        limit: Some(1),
        session: None,
    };
    let result = search(&p, &args).unwrap();
    assert_eq!(result["matches"].as_array().unwrap().len(), 1);
    assert_eq!(result["omitted"], 1);
    assert_eq!(result["_detail"]["matches"].as_array().unwrap().len(), 2);
    assert_eq!(result["complete"], true);
}
#[test]
fn files_are_literal_and_unsafe_scope_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a[1].txt"), "text").unwrap();
    let p = Project::new(dir.path()).unwrap();
    let mut args = SearchArgs {
        external_scope: None,
        context_lines: None,
        min_lines: None,
        globs: None,
        kind: Kind::Files,
        query: "[1]".into(),
        scope: None,
        limit: None,
        session: None,
    };
    assert_eq!(
        search(&p, &args).unwrap()["paths"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    args.scope = Some("../outside".into());
    assert_eq!(search(&p, &args).unwrap_err().0, "PATH");
}

#[test]
fn globs_alone_list_files_without_a_query() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.rs"), "text\n").unwrap();
    fs::write(dir.path().join("b.txt"), "text\n").unwrap();
    let p = Project::new(dir.path()).unwrap();
    let args: SearchArgs = serde_json::from_value(json!({
            "kind":"files","globs":["*.rs"],"scope":"."}))
    .expect("SearchArgs");
    let result = search(&p, &args).unwrap();
    let paths = result["paths"].as_array().unwrap();
    assert_eq!(paths.len(), 1);
    assert!(paths[0].as_str().unwrap().ends_with("a.rs"));
    assert_eq!(result["complete"], true);
}

#[test]
fn large_files_make_results_explicitly_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("large.txt"),
        vec![b'x'; 2 * 1024 * 1024 + 1],
    )
    .unwrap();
    fs::write(dir.path().join("small.txt"), "needle\n").unwrap();
    let p = Project::new(dir.path()).unwrap();
    let args = SearchArgs {
        external_scope: None,
        context_lines: None,
        min_lines: None,
        globs: None,
        kind: Kind::Text,
        query: "needle".into(),
        scope: None,
        limit: None,
        session: None,
    };
    let result = search(&p, &args).unwrap();
    assert_eq!(result["matches"].as_array().unwrap().len(), 1);
    assert_eq!(result["complete"], false);
    assert_eq!(result["_complete"], false);
    assert_eq!(result["skipped"], 1);
    assert_eq!(result["skippedFiles"][0]["path"], "large.txt");
    assert_eq!(result["skippedFiles"][0]["reason"], "read_error");
    for scope in [None, Some("."), Some("")] {
        let rooted = SearchArgs {
            scope: scope.map(str::to_owned),
            query: args.query.clone(),
            ..serde_json::from_value(json!({"kind":"text","query":"needle"})).unwrap()
        };
        assert_eq!(search(&p, &rooted).unwrap()["matches"], result["matches"]);
    }
    assert_eq!(result["diagnostics"][0]["code"], "SEARCH_INCOMPLETE");
}

#[test]
fn globs_filter_files_and_file_size_without_skipping() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/App.vue"), "needle\n").unwrap();
    fs::write(dir.path().join("src/main.ts"), "needle\n").unwrap();
    fs::write(dir.path().join("README.md"), "needle\n").unwrap();
    let p = Project::new(dir.path()).unwrap();
    let files = search(
        &p,
        &SearchArgs {
            external_scope: None,
            context_lines: None,
            min_lines: None,
            globs: Some(vec!["*.vue".into()]),
            kind: Kind::Files,
            query: ".vue".into(),
            scope: None,
            limit: None,
            session: None,
        },
    )
    .unwrap();
    let paths = files["paths"].as_array().unwrap();
    assert_eq!(paths.len(), 1);
    assert!(paths[0].as_str().unwrap().ends_with("App.vue"));
    assert_eq!(files["skipped"], 0);
    assert_eq!(files["complete"], true);
    assert_eq!(files["globs"], json!(["*.vue"]));
    let sizes = search(
        &p,
        &SearchArgs {
            external_scope: None,
            context_lines: None,
            min_lines: Some(0),
            globs: Some(vec!["src/**".into()]),
            kind: Kind::FileSize,
            query: String::new(),
            scope: None,
            limit: None,
            session: None,
        },
    )
    .unwrap();
    assert_eq!(sizes["files"].as_array().unwrap().len(), 2);
    let text = search(
        &p,
        &SearchArgs {
            external_scope: None,
            context_lines: None,
            min_lines: None,
            globs: Some(vec!["*.vue".into()]),
            kind: Kind::Text,
            query: "needle".into(),
            scope: None,
            limit: None,
            session: None,
        },
    )
    .unwrap();
    assert_eq!(text["matches"].as_array().unwrap().len(), 1);
    assert_eq!(text["bytesRead"], 7);
    assert_eq!(text["skipped"], 0);
    assert_eq!(text["complete"], true);
    assert_eq!(text["globs"], json!(["*.vue"]));
}

#[test]
fn text_globs_filter_extensions_before_reading_and_preserve_exclusions() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("Views/Orders")).unwrap();
    fs::create_dir(dir.path().join("node_modules")).unwrap();
    fs::write(dir.path().join("Views/Orders/Index.cshtml"), "needle\n").unwrap();
    fs::write(dir.path().join("Views/Orders/app.js"), "needle\n").unwrap();
    fs::write(dir.path().join("Views/Orders/ignored.bin"), [0xff; 128]).unwrap();
    fs::write(dir.path().join("node_modules/vendor.cshtml"), "needle\n").unwrap();
    let p = Project::new(dir.path()).unwrap();
    let args = SearchArgs {
        external_scope: None,
        context_lines: None,
        min_lines: None,
        globs: Some(vec!["*.cshtml".into(), "*.js".into()]),
        kind: Kind::Text,
        query: "needle".into(),
        scope: None,
        limit: None,
        session: None,
    };
    let result = search(&p, &args).unwrap();
    assert_eq!(result["matches"].as_array().unwrap().len(), 2);
    assert_eq!(result["bytesRead"], 14);
    assert_eq!(result["skipped"], 0);
    assert_eq!(result["complete"], true);
    let scoped = search(
        &p,
        &SearchArgs {
            scope: Some("Views/Orders".into()),
            globs: Some(vec!["Views/**/*.cshtml".into()]),
            ..args
        },
    )
    .unwrap();
    assert_eq!(scoped["matches"].as_array().unwrap().len(), 1);
    assert_eq!(scoped["bytesRead"], 7);
}

fn text_args(scope: Option<&str>) -> SearchArgs {
    SearchArgs {
        external_scope: None,
        context_lines: None,
        min_lines: None,
        globs: None,
        kind: Kind::Text,
        query: "needle".into(),
        scope: scope.map(str::to_owned),
        limit: None,
        session: None,
    }
}

#[test]
fn harnessignore_excludes_and_supports_negation_without_incomplete_results() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("generated")).unwrap();
    fs::write(dir.path().join("generated/drop.txt"), "needle\n").unwrap();
    fs::write(dir.path().join("generated/keep.txt"), "needle\n").unwrap();
    fs::write(dir.path().join("src.txt"), "needle\n").unwrap();
    fs::write(
        dir.path().join(".harnessignore"),
        "generated/*\n!generated/keep.txt\n",
    )
    .unwrap();
    let project = Project::new(dir.path()).unwrap();
    let result = search(&project, &text_args(None)).unwrap();
    let paths: Vec<_> = result["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["file"].as_str().unwrap())
        .collect();
    assert_eq!(paths.len(), 2);
    assert!(paths.iter().any(|path| path.ends_with("keep.txt")));
    assert!(paths.iter().any(|path| *path == "src.txt"));
    assert_eq!(result["complete"], true);
    assert_eq!(result["skipped"], 0);
    assert_eq!(result["exclusionPolicy"]["scopeOverride"], false);
    assert_eq!(result["exclusionPolicy"]["customPatterns"], 2);
}

#[test]
fn explicit_scope_overrides_harnessignore_with_warning() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("generated")).unwrap();
    fs::write(dir.path().join("generated/drop.txt"), "needle\n").unwrap();
    fs::write(dir.path().join(".harnessignore"), "generated/\n").unwrap();
    let project = Project::new(dir.path()).unwrap();
    let result = search(&project, &text_args(Some("generated"))).unwrap();
    assert_eq!(result["matches"].as_array().unwrap().len(), 1);
    assert_eq!(result["warnings"][0]["code"], "SCOPE_OVERRIDES_EXCLUSIONS");
    assert_eq!(result["exclusionPolicy"]["customFile"], Value::Null);
    assert_eq!(result["complete"], true);
}

#[test]
fn defaults_and_secrets_cannot_be_re_included() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("node_modules")).unwrap();
    fs::create_dir_all(dir.path().join("target")).unwrap();
    fs::write(dir.path().join("node_modules/drop.txt"), "needle\n").unwrap();
    fs::write(dir.path().join("target/drop.txt"), "needle\n").unwrap();
    fs::write(dir.path().join("src.txt"), "needle\n").unwrap();
    fs::write(
        dir.path().join(".harnessignore"),
        "!node_modules/\n!target/\n!node_modules/drop.txt\n",
    )
    .unwrap();
    let project = Project::new(dir.path()).unwrap();
    let result = search(&project, &text_args(None)).unwrap();
    let paths: Vec<_> = result["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["file"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec!["src.txt"]);
    assert_eq!(result["complete"], true);
}

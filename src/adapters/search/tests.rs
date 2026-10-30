use super::*;
use harness_domain::contracts::Kind;
use serde_json::json;
use std::fs;

#[test]
fn bm25_returns_default_context_and_honors_scope_and_ignore() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    let source = "fn cursorHandler() {\n    paginate();\n}\n";
    fs::write(dir.path().join("src/a.rs"), source).unwrap();
    fs::write(dir.path().join("outside.rs"), "cursor paginate\n").unwrap();
    fs::write(dir.path().join("src/secret.rs"), "cursor paginate\n").unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args: SearchArgs =
        serde_json::from_value(json!({"query":"cursor paginate", "scope":"src"})).unwrap();
    let result = search(&project, &args).unwrap();
    assert_eq!(result["retrieval"]["ranking"], "bm25");
    assert_eq!(result["contextLines"], 2);
    for row in result["matches"].as_array().unwrap() {
        assert_eq!(row["file"].as_str().unwrap().replace('\\', "/"), "src/a.rs");
        assert_eq!(row["context"]["text"], source);
    }
    assert!(!dir.path().join(".codegraph").exists());
}

#[test]
fn bm25_automatically_recovers_terms_and_preserves_source_context() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.rs"), "fn pagination() {}\r\n").unwrap();
    let source = "fn cursorPagination() {}\r\n";
    fs::write(dir.path().join("b.rs"), source).unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args: SearchArgs =
        serde_json::from_value(json!({"query":"cursor pagination","contextLines":0,"limit":1}))
            .unwrap();
    let result = search(&project, &args).unwrap();
    assert_eq!(result["retrieval"]["ranking"], "bm25");
    assert_eq!(result["matches"][0]["file"], "b.rs", "{result}");
    assert_eq!(result["matches"][0]["context"]["text"], source);
    assert_eq!(
        result["matches"][0]["hash"],
        harness_domain::hash(source.as_bytes())
    );
    assert_eq!(result["_detail"]["matches"].as_array().unwrap().len(), 2);
    assert_eq!(result["complete"], true);
    assert_eq!(result["omitted"], 1);
}

#[test]
fn bm25_preserves_exact_regex_case_and_single_identifier_searches() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("a.rs"),
        "cursor pagination\nfn cursorPagination() {}\n",
    )
    .unwrap();
    let project = Project::new(dir.path()).unwrap();
    for input in [
        json!({"query":"cursor pagination"}),
        json!({"query":"pagination cursor","regex":true}),
        json!({"query":"pagination cursor","caseSensitive":false}),
        json!({"query":"missingIdentifier"}),
        json!({"query":"cursor(pagination)"}),
    ] {
        let args: SearchArgs = serde_json::from_value(input).unwrap();
        let result = search(&project, &args).unwrap();
        assert!(result.get("retrieval").is_none(), "{result}");
    }
}

#[test]
fn bm25_respects_globs_and_rejects_substring_false_positives() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.rs"), "fn concatenate() {}\n").unwrap();
    fs::write(dir.path().join("b.js"), "cat dog\n").unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args: SearchArgs =
        serde_json::from_value(json!({"query":"cat dog","globs":["*.rs"]})).unwrap();
    let result = search(&project, &args).unwrap();
    assert_eq!(result["retrieval"]["ranking"], "bm25");
    assert_eq!(result["matches"].as_array().unwrap().len(), 0);
    assert_eq!(result["complete"], true);
}

#[test]
fn ast_search_matches_multiline_calls_but_not_strings_comments_or_other_languages() {
    let dir = tempfile::tempdir().unwrap();
    let source = "// fetch(fake)\r\nconst label = 'fetch(fake)';\r\nfetch(\r\n  url\r\n);\r\nfetch(other);\r\n";
    fs::write(dir.path().join("a.js"), source).unwrap();
    fs::write(dir.path().join("wrong.py"), "fetch(other)").unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args: SearchArgs = serde_json::from_value(json!({
        "query":"fetch($URL)","ast":"javascript","contextLines":1,"limit":1
    }))
    .unwrap();
    let result = search(&project, &args).unwrap();
    let matches = result["_detail"]["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 2, "{result}");
    assert_eq!(matches[0]["line"], 3);
    assert_eq!(matches[0]["endLine"], 5);
    assert_eq!(matches[0]["text"], "fetch(\r\n  url\r\n)");
    assert_eq!(matches[0]["context"]["start"], 2);
    assert_eq!(matches[0]["context"]["end"], 6);
    assert_eq!(
        matches[0]["context"]["text"],
        &source[source.find("const").unwrap()..]
    );
    assert_eq!(matches[0]["hash"], harness_domain::hash(source.as_bytes()));
    let start = matches[0]["startByte"].as_u64().unwrap() as usize;
    let end = matches[0]["endByte"].as_u64().unwrap() as usize;
    assert_eq!(&source[start..end], matches[0]["text"].as_str().unwrap());
    assert_eq!(result["omitted"], 1);
    assert_eq!(result["parsedFiles"], 1);
    assert_eq!(result["complete"], true);
}

#[test]
fn ast_search_supports_languages_and_library_metavariables_without_an_index() {
    for (language, file, source, query) in [
        ("rust", "a.rs", "fn f() { send(1, 2); }", "send($$$ARGS)"),
        (
            "typescript",
            "a.ts",
            "const x: number = send(1);",
            "send($ARG)",
        ),
        (
            "tsx",
            "a.tsx",
            "const x = <Button onClick={send(1)} />;",
            "send($ARG)",
        ),
        ("python", "a.py", "send(1)\n", "send($ARG)"),
        (
            "csharp",
            "a.cs",
            "class C { void M() { Send(1); } }",
            "Send($ARG)",
        ),
        (
            "go",
            "a.go",
            "package main\nfunc f() { send(1) }",
            "send($ARG)",
        ),
        (
            "html",
            "a.html",
            "<div><span>Hello</span></div>",
            "<span>$TEXT</span>",
        ),
        ("css", "a.css", "a { color: red; }", "a { color: $VALUE; }"),
        ("json", "a.json", "{\"ok\": true}", "{\"ok\": $VALUE}"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(file), source).unwrap();
        let project = Project::new(dir.path()).unwrap();
        let args: SearchArgs =
            serde_json::from_value(json!({"ast":language,"query":query})).unwrap();
        let result =
            search(&project, &args).unwrap_or_else(|error| panic!("{language}: {error:?}"));
        assert_eq!(
            result["matches"].as_array().unwrap().len(),
            1,
            "{language}: {result}"
        );
        assert_eq!(result["complete"], true);
        assert!(result["matches"][0]["context"]["text"].is_string());
        assert!(!dir.path().join(".codegraph").exists());
    }
}

#[test]
fn ast_search_rejects_bad_patterns_and_conflicting_options_before_scanning() {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::new(dir.path()).unwrap();
    for input in [
        json!({"ast":"not-a-language","query":"foo($X)"}),
        json!({"ast":"javascript","query":"???"}),
        json!({"ast":"javascript","query":"foo($X)","regex":false}),
        json!({"ast":"javascript","query":"foo($X)","caseSensitive":true}),
        json!({"kind":"syntax","ast":"javascript","query":"foo($X)"}),
        json!({"kind":"files","ast":"javascript","query":"foo($X)"}),
    ] {
        let args: SearchArgs = serde_json::from_value(input).unwrap();
        assert_eq!(search(&project, &args).unwrap_err().0, "ARGUMENT");
    }
}

#[test]
fn ast_search_reports_parse_errors_and_preserves_ignore_policy_and_globs() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.js"), "send(1);").unwrap();
    fs::write(dir.path().join("bad.js"), "function { send(2);").unwrap();
    fs::write(dir.path().join("ignored.js"), "send(3);").unwrap();
    fs::write(dir.path().join("filtered.js"), "send(4);").unwrap();
    fs::write(dir.path().join("secret.js"), "send(5);").unwrap();
    fs::write(dir.path().join(".gitignore"), "ignored.js\n").unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args: SearchArgs = serde_json::from_value(json!({
        "ast":"javascript","query":"send(\n$ARG\n)","globs":["a.js","bad.js","ignored.js","secret.js"]
    })).unwrap();
    let result = search(&project, &args).unwrap();
    assert_eq!(result["matches"].as_array().unwrap().len(), 1, "{result}");
    assert_eq!(result["matches"][0]["file"], "a.js");
    assert_eq!(result["parseErrors"], 1);
    assert_eq!(result["skippedFiles"][0]["reason"], "parse_error");
    assert_eq!(result["complete"], false);
}

#[test]
fn ripgrep_keeps_literal_default_and_supports_regex_and_unicode_case_folding() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("a.txt"),
        "foo.bar\nfooXbar\nÉXITO\nfoo42 foo43",
    )
    .unwrap();
    let project = Project::new(dir.path()).unwrap();
    for (query, regex, case_sensitive, lines) in [
        ("foo.bar", None, None, vec![1]),
        ("foo.bar", Some(true), None, vec![1, 2]),
        ("éxito", None, None, vec![]),
        ("éxito", None, Some(false), vec![3]),
        (r"^foo\d+", Some(true), None, vec![4]),
        (r"(?i)^éxito$", Some(true), None, vec![3]),
        (r"^missing$", Some(true), None, vec![]),
    ] {
        let args: SearchArgs = serde_json::from_value(json!({
            "query":query, "regex":regex, "caseSensitive":case_sensitive
        }))
        .unwrap();
        let result = search(&project, &args).unwrap();
        let found: Vec<u64> = result["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["line"].as_u64().unwrap())
            .collect();
        assert_eq!(found, lines, "query={query}, regex={regex:?}");
        assert_eq!(result["complete"], true);
    }
}

#[test]
fn ripgrep_regex_context_preserves_bom_crlf_and_final_line() {
    let dir = tempfile::tempdir().unwrap();
    let source = "\u{feff}start\r\nCall(42);  \r\nCall(7);";
    fs::write(dir.path().join("a.txt"), source).unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args: SearchArgs = serde_json::from_value(json!({
        "query":r"Call\(\d+\)", "regex":true, "contextLines":1
    }))
    .unwrap();
    let result = search(&project, &args).unwrap();
    assert_eq!(result["matches"][0]["line"], 2);
    assert_eq!(result["matches"][0]["context"]["text"], source);
    assert_eq!(
        result["matches"][0]["hash"],
        harness_domain::hash(source.as_bytes())
    );
    assert_eq!(result["_detail"]["matches"][1]["text"], "Call(7);");
    assert_eq!(result["matches"][1]["context"]["start"], 2);
    assert_eq!(result["matches"][1]["context"]["end"], 3);
}

#[test]
fn ripgrep_rejects_invalid_regex_and_text_options_on_other_kinds() {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::new(dir.path()).unwrap();
    for input in [
        json!({"query":"[", "regex":true}),
        json!({"query":r"(?<=foo)bar", "regex":true}),
        json!({"kind":"files", "query":"foo", "regex":false}),
        json!({"kind":"syntax", "query":"function", "caseSensitive":true}),
    ] {
        let args: SearchArgs = serde_json::from_value(input).unwrap();
        assert_eq!(search(&project, &args).unwrap_err().0, "ARGUMENT");
    }
}

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
    assert_eq!(
        result["_detail"]["exclusionPolicy"]["customFile"],
        ".harnessignore"
    );
    assert!(result["_detail"]["exclusionPolicy"]["customPatterns"].is_null());
    args.scope = Some("visible.txt".into());
    let scoped = search(&project, &args).unwrap();
    assert_eq!(scoped["_detail"]["exclusionPolicy"]["customFile"], ".harnessignore");
    assert!(scoped["_detail"]["exclusionPolicy"]["customPatterns"].is_null());
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
            override_project_exclusions: None,
        ast: None,
        regex: None,
        case_sensitive: None,
        external_scope: None,
        context_lines: None,
        min_lines: None,
        globs: None,
        kind: Kind::Text,
        query: "needle".into(),
        scope: None,
        limit: Some(1),
        session: None,
        cursor: None,
        offset: None,
        only_matching: None,
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
            override_project_exclusions: None,
        ast: None,
        regex: None,
        case_sensitive: None,
        external_scope: None,
        context_lines: None,
        min_lines: None,
        globs: None,
        kind: Kind::Files,
        query: "[1]".into(),
        scope: None,
        limit: None,
        session: None,
        cursor: None,
        offset: None,
        only_matching: None,
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
fn recognized_large_binary_signatures_are_skipped_but_misleading_extensions_are_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    let mut pe = b"MZ".to_vec();
    pe.resize(2 * 1024 * 1024 + 1, 0x90);
    fs::write(dir.path().join("tool.exe"), &pe).unwrap();
    let mut woff = b"wOF2".to_vec();
    woff.resize(2 * 1024 * 1024 + 1, 0);
    fs::write(dir.path().join("font.woff2"), &woff).unwrap();
    fs::write(dir.path().join("misnamed.exe"), vec![b'x'; 2 * 1024 * 1024 + 1]).unwrap();
    fs::write(dir.path().join("small.txt"), "needle\\n").unwrap();

    let project = Project::new(dir.path()).unwrap();
    let result = search(&project, &text_args(None)).unwrap();

    assert_eq!(result["matches"].as_array().unwrap().len(), 1);
    assert_eq!(result["binarySkipped"], 2);
    assert_eq!(result["skipped"], 1);
    assert_eq!(result["skippedFiles"][0]["path"], "misnamed.exe");
    assert_eq!(result["skippedFiles"][0]["reason"], "file_size_limit");
    assert_eq!(result["complete"], false);
    assert_eq!(result["diagnostics"][0]["code"], "SEARCH_INCOMPLETE");
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
            override_project_exclusions: None,
        ast: None,
        regex: None,
        case_sensitive: None,
        external_scope: None,
        context_lines: None,
        min_lines: None,
        globs: None,
        kind: Kind::Text,
        query: "needle".into(),
        scope: None,
        limit: None,
        session: None,
        cursor: None,
        offset: None,
        only_matching: None,
    };
    let result = search(&p, &args).unwrap();
    assert_eq!(result["matches"].as_array().unwrap().len(), 1);
    assert_eq!(result["complete"], false);
    assert_eq!(result["_complete"], false);
    assert_eq!(result["skipped"], 1);
    assert_eq!(result["skippedFiles"][0]["path"], "large.txt");
    assert_eq!(result["skippedFiles"][0]["reason"], "file_size_limit");
    for scope in [None, Some("."), Some("")] {
        let rooted = SearchArgs {
            override_project_exclusions: None,
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
            override_project_exclusions: None,
            ast: None,
            regex: None,
            case_sensitive: None,
            external_scope: None,
            context_lines: None,
            min_lines: None,
            globs: Some(vec!["*.vue".into()]),
            kind: Kind::Files,
            query: ".vue".into(),
            scope: None,
            limit: None,
            session: None,
            cursor: None,
            offset: None,
            only_matching: None,
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
            override_project_exclusions: None,
            ast: None,
            regex: None,
            case_sensitive: None,
            external_scope: None,
            context_lines: None,
            min_lines: Some(0),
            globs: Some(vec!["src/**".into()]),
            kind: Kind::FileSize,
            query: String::new(),
            scope: None,
            limit: None,
            session: None,
            cursor: None,
            offset: None,
            only_matching: None,
        },
    )
    .unwrap();
    assert_eq!(sizes["files"].as_array().unwrap().len(), 2);
    let text = search(
        &p,
        &SearchArgs {
            override_project_exclusions: None,
            ast: None,
            regex: None,
            case_sensitive: None,
            external_scope: None,
            context_lines: None,
            min_lines: None,
            globs: Some(vec!["*.vue".into()]),
            kind: Kind::Text,
            query: "needle".into(),
            scope: None,
            limit: None,
            session: None,
            cursor: None,
            offset: None,
            only_matching: None,
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
            override_project_exclusions: None,
        ast: None,
        regex: None,
        case_sensitive: None,
        external_scope: None,
        context_lines: None,
        min_lines: None,
        globs: Some(vec!["*.cshtml".into(), "*.js".into()]),
        kind: Kind::Text,
        query: "needle".into(),
        scope: None,
        limit: None,
        session: None,
        cursor: None,
        offset: None,
        only_matching: None,
    };
    let result = search(&p, &args).unwrap();
    assert_eq!(result["matches"].as_array().unwrap().len(), 2);
    assert_eq!(result["bytesRead"], 14);
    assert_eq!(result["skipped"], 0);
    assert_eq!(result["complete"], true);
    let scoped = search(
        &p,
        &SearchArgs {
            override_project_exclusions: None,
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
            override_project_exclusions: None,
        ast: None,
        regex: None,
        case_sensitive: None,
        external_scope: None,
        context_lines: None,
        min_lines: None,
        globs: None,
        kind: Kind::Text,
        query: "needle".into(),
        scope: scope.map(str::to_owned),
        limit: None,
        session: None,
        cursor: None,
        offset: None,
        only_matching: None,
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
    assert_eq!(result["_detail"]["exclusionPolicy"]["scopeOverride"], false);
    assert_eq!(result["_detail"]["exclusionPolicy"]["customPatterns"], 2);
}

#[test]
fn explicit_scope_overrides_harnessignore_with_warning() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("generated")).unwrap();
    fs::write(dir.path().join("generated/drop.txt"), "needle\n").unwrap();
    fs::write(dir.path().join(".harnessignore"), "generated/\n").unwrap();
    let project = Project::new(dir.path()).unwrap();
    let mut args = text_args(Some("generated"));
    args.override_project_exclusions = Some(true);
    let result = search(&project, &args).unwrap();
    assert_eq!(result["matches"].as_array().unwrap().len(), 1);
    assert_eq!(result["warnings"][0]["code"], "SCOPE_OVERRIDES_EXCLUSIONS");
    assert_eq!(
        result["_detail"]["exclusionPolicy"]["customFile"],
        Value::Null
    );
    assert_eq!(result["complete"], true);
}

#[test]
fn explicit_scope_respects_project_ignore_unless_override_requested() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("generated")).unwrap();
    fs::write(dir.path().join("generated/drop.txt"), "needle\n").unwrap();
    fs::write(dir.path().join(".harnessignore"), "generated/\n").unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args = serde_json::from_value(json!({"query":"needle", "scope":"generated"})).unwrap();
    let result = search(&project, &args).unwrap();
    assert!(result["matches"].as_array().unwrap().is_empty());
    assert!(result["warnings"].as_array().unwrap().is_empty());
    let args = serde_json::from_value(json!({"query":"needle", "scope":"generated", "overrideProjectExclusions":true})).unwrap();
    let result = search(&project, &args).unwrap();
    assert_eq!(result["matches"].as_array().unwrap().len(), 1);
    assert_eq!(result["warnings"][0]["code"], "SCOPE_OVERRIDES_EXCLUSIONS");
}

#[test]
fn scope_inside_hard_excluded_directory_is_rejected_even_with_override() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("node_modules/nested")).unwrap();
    fs::write(dir.path().join("node_modules/nested/file.txt"), "needle").unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args: SearchArgs = serde_json::from_value(json!({"query":"needle", "scope":"node_modules/nested", "overrideProjectExclusions":true})).unwrap();
    assert_eq!(search(&project, &args).unwrap_err().0, "PATH");
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

#[test]
fn compact_text_truncation_keeps_unicode_boundary_detail_and_file_order() {
    let dir = tempfile::tempdir().unwrap();
    let long_line = format!("needle{}\n", "😀".repeat(300));
    // Create in reverse lexical order: the public result order stays stable.
    fs::write(dir.path().join("z.txt"), "needle last\n").unwrap();
    fs::write(dir.path().join("a.txt"), &long_line).unwrap();
    let project = Project::new(dir.path()).unwrap();

    let result = search(&project, &text_args(None)).unwrap();
    let matches = result["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0]["file"], "a.txt");
    assert_eq!(matches[1]["file"], "z.txt");
    let compact_text = matches[0]["text"].as_str().unwrap();
    assert_eq!(compact_text, format!("needle{}", "😀".repeat(247)));
    assert_eq!(compact_text.encode_utf16().count(), 500);
    assert_eq!(result["truncated"], true);
    assert_eq!(result["_detail"]["matches"][0]["text"], long_line);
    assert_eq!(result["complete"], true);
}

#[test]
fn file_size_min_lines_excludes_files_equal_to_the_threshold() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("one-line.txt"), "one\n").unwrap();
    fs::write(dir.path().join("two-lines.txt"), "one\ntwo").unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args: SearchArgs = serde_json::from_value(json!({
        "kind":"file_size", "query":"", "minLines":1
    }))
    .unwrap();

    let result = search(&project, &args).unwrap();
    let files = result["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["path"], "two-lines.txt");
    assert_eq!(files[0]["lines"], 2);
    assert_eq!(result["complete"], true);
    assert_eq!(result["skipped"], 0);
}

#[test]
fn match_metadata_budget_rejects_an_oversized_row_without_cutting_saved_source() {
    let dir = tempfile::tempdir().unwrap();
    let quote_heavy_line = format!("needle{}\n", "\"".repeat(1_100_000));
    assert!(quote_heavy_line.len() < 2 * 1024 * 1024);
    fs::write(dir.path().join("a.txt"), &quote_heavy_line).unwrap();
    fs::write(dir.path().join("b.txt"), &quote_heavy_line).unwrap();
    let project = Project::new(dir.path()).unwrap();

    let result = search(&project, &text_args(None)).unwrap();
    assert_eq!(result["limited"], true);
    assert_eq!(result["complete"], false);
    assert_eq!(result["diagnostics"][0]["code"], "SEARCH_INCOMPLETE");
    let detail_matches = result["_detail"]["matches"].as_array().unwrap();
    assert_eq!(detail_matches.len(), 1);
    assert_eq!(detail_matches[0]["file"], "a.txt");
    assert_eq!(detail_matches[0]["text"], quote_heavy_line);
    assert_eq!(result["skippedFiles"][0]["path"], "b.txt");
    assert_eq!(result["skippedFiles"][0]["reason"], "metadata_budget");
}

#[test]
fn only_matching_extracts_matched_tokens_and_capture_groups_with_deduplicated_list() {
    let dir = tempfile::tempdir().unwrap();
    let source = concat!(
        "using (var cmd = new SqlCommand(\"PA_Caja_Apertura\", cn)) {}\n",
        "using (var cmd = new SqlCommand(\"PA_Caja_Cierre\", cn)) {}\n",
        "using (var cmd = new SqlCommand(\"PA_Caja_Apertura\", cn)) {}\n",
    );
    fs::write(dir.path().join("P_Caja.cs"), source).unwrap();
    let project = Project::new(dir.path()).unwrap();
    let args: SearchArgs = serde_json::from_value(json!({
        "kind": "text",
        "query": r#"SqlCommand\("([^"]+)""#,
        "regex": true,
        "onlyMatching": true
    }))
    .unwrap();
    let result = search(&project, &args).unwrap();
    assert_eq!(result["onlyMatching"], true);
    assert_eq!(
        result["extracted"],
        json!(["PA_Caja_Apertura", "PA_Caja_Cierre"])
    );
    let matches = result["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 3);
    assert_eq!(matches[0]["text"], "PA_Caja_Apertura");
    assert_eq!(matches[1]["text"], "PA_Caja_Cierre");
    assert_eq!(matches[2]["text"], "PA_Caja_Apertura");
}

#[test]
fn diacritic_and_windows1252_fallback_matches_accents_entities_and_unaccented_text() {
    let dir = tempfile::tempdir().unwrap();
    // 1. Unaccented text in source matched by accented query ("Garantías" -> "Garantias")
    fs::write(dir.path().join("view.cshtml"), "<label>Garantias</label>\n<label>Tipo &oacute;peracion</label>\n").unwrap();
    // 2. Windows-1252 encoded file with accented text ("Tipo operación")
    fs::write(dir.path().join("legacy.js"), b"// Tipo operaci\xf3n\n").unwrap();
    let project = Project::new(dir.path()).unwrap();

    let res1 = search(&project, &serde_json::from_value(json!({"kind":"text","query":"Garantías"})).unwrap()).unwrap();
    assert_eq!(res1["matches"].as_array().unwrap().len(), 1, "{res1}");
    assert_eq!(res1["matches"][0]["file"], "view.cshtml");

    let res2 = search(&project, &serde_json::from_value(json!({"kind":"text","query":"Tipo operación"})).unwrap()).unwrap();
    assert_eq!(res2["matches"].as_array().unwrap().len(), 2, "{res2}");
}

#![cfg(windows)]
use agent_harness::adapters;
use harness_domain::{
    contracts::{ReadArgs, ReplaceArgs, Request},
    execution::OperationContext,
    hash,
};
use serde_json::Value;

#[test]
fn multi_chunk_publishes_once_and_rejects_invalid_candidate_without_writes() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let file = root.path().join("multi.rs");
    let before = "fn first() { let x = 1; }\r\nfn second() { let y = 2; }\r\n";
    std::fs::write(&file, before).unwrap();
    let execute = |chunks: Value| {
        let args: ReplaceArgs = serde_json::from_value(serde_json::json!({
            "path":"multi.rs", "hash":hash(before.as_bytes()), "chunks":chunks
        }))
        .unwrap();
        app.execute(Request::Replace(args))
    };
    for chunks in [
        serde_json::json!([{"oldText":"1","newText":"10"},{"oldText":"2","newText":"{"}]),
        serde_json::json!([{"oldText":"1","newText":"10"},{"oldText":"missing","newText":"20"}]),
    ] {
        let result = execute(chunks);
        assert_eq!(result["state"], "error", "{result}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
    }
    let result = execute(serde_json::json!([
        {"oldText":"2","newText":"20"},{"oldText":"1","newText":"10"}
    ]));
    let expected = "fn first() { let x = 10; }\r\nfn second() { let y = 20; }\r\n";
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["verified"], true);
    assert_eq!(result["data"]["hash"], hash(expected.as_bytes()));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), expected);
    let stale = execute(serde_json::json!([{"oldText":"10","newText":"30"}]));
    assert_eq!(stale["diagnostics"][0]["code"], "STALE");
}

fn replace(
    app: &adapters::NativeApplication,
    path: &str,
    text: &str,
    old: &str,
    new: &str,
) -> Value {
    app.execute(Request::Replace(ReplaceArgs {
        path: path.into(),
        hash: hash(text.as_bytes()),
        chunks: None,
        old_text: old.into(),
        new_text: new.into(),
        allow_unvalidated_text: false,
        preview: false,
        session: None,
    }))
}

#[test]
fn publishes_valid_rust_json_and_text_preserving_bytes() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    for (path, text, old, new, expected) in [
        (
            "sample.rs",
            "\u{feff}fn sample() { let x = 1; }\r\n",
            "1",
            "2",
            "\u{feff}fn sample() { let x = 2; }\r\n",
        ),
        ("sample.json", "{\"x\":1}\n", "1", "2", "{\"x\":2}\n"),
        ("sample.js", "const x = 1;\n", "1", "2", "const x = 2;\n"),
        (
            "sample.ts",
            "const x: number = 1;\n",
            "1",
            "2",
            "const x: number = 2;\n",
        ),
        (
            "sample.txt",
            "🙂\r\na\r\nb\r\n",
            "a\nb",
            "c\nd",
            "🙂\r\nc\r\nd\r\n",
        ),
    ] {
        std::fs::write(root.path().join(path), text).unwrap();
        let result = replace(&app, path, text, old, new);
        assert_eq!(result["state"], "ok", "{result}");
        assert_eq!(result["data"]["changed"], true);
        assert_eq!(result["data"]["verified"], true);
        assert_eq!(result["data"]["hash"], hash(expected.as_bytes()));
        assert_eq!(
            std::fs::read(root.path().join(path)).unwrap(),
            expected.as_bytes()
        );
    }
    assert!(!std::fs::read_dir(root.path()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".harness-")
    }));
}

#[test]
fn validation_stale_and_cancellation_leave_original_untouched() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    for (path, text, old, new, code) in [
        ("bad.rs", "fn sample() {}", "{}", "{", "SYNTAX_ERROR"),
        ("bad.json", "{\"x\":1}", "1", "[", "SYNTAX_ERROR"),
        ("bad.js", "const a = 1;", "1", "const", "SYNTAX_ERROR"),
        ("bad.ts", "const a: number = 1;", "1", "{", "SYNTAX_ERROR"),
        ("overlap.txt", "aaa", "aa", "b", "MATCH"),
    ] {
        std::fs::write(root.path().join(path), text).unwrap();
        let result = replace(&app, path, text, old, new);
        assert_eq!(result["diagnostics"][0]["code"], code, "{result}");
        assert_eq!(
            std::fs::read_to_string(root.path().join(path)).unwrap(),
            text
        );
    }
    std::fs::write(root.path().join("file.txt"), "current").unwrap();
    assert_eq!(
        replace(&app, "file.txt", "stale", "stale", "next")["diagnostics"][0]["code"],
        "STALE"
    );
    let context = OperationContext::default();
    context.cancel();
    let result = app.execute_with_context(
        Request::Replace(ReplaceArgs {
            path: "file.txt".into(),
            hash: hash(b"current"),
            chunks: None,
            old_text: "current".into(),
            new_text: "next".into(),
            allow_unvalidated_text: false,
            preview: false,
            session: None,
        }),
        context,
    );
    assert_eq!(result["state"], "cancelled");
    assert_eq!(
        std::fs::read_to_string(root.path().join("file.txt")).unwrap(),
        "current"
    );
}

#[test]
fn protects_instructions_internal_paths_secrets_and_hardlinks() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    for path in [
        "AGENTS.md",
        ".agents/harness/data.txt",
        ".git/config",
        ".env",
        "secrets.txt",
    ] {
        let file = root.path().join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "old").unwrap();
        assert_eq!(
            replace(&app, path, "old", "old", "new")["diagnostics"][0]["code"],
            "PATH"
        );
        assert_eq!(std::fs::read_to_string(file).unwrap(), "old");
    }
    std::fs::write(root.path().join("original.txt"), "old").unwrap();
    std::fs::hard_link(
        root.path().join("original.txt"),
        root.path().join("alias.txt"),
    )
    .unwrap();
    assert_eq!(
        replace(&app, "alias.txt", "old", "old", "new")["diagnostics"][0]["code"],
        "PATH"
    );
    let result = app.execute(Request::Read(ReadArgs {
        external_scope: None,
        path: "original.txt".into(),
        start: 1,
        end: 1,
        session: None,
    }));
    assert_eq!(result["data"]["text"], "old");
}

#[test]
fn publication_failure_preserves_candidate_for_explicit_recovery() {
    use std::os::windows::fs::OpenOptionsExt;
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("file.txt");
    std::fs::write(&file, "old").unwrap();
    let held = std::fs::File::options()
        .read(true)
        .share_mode(3)
        .open(&file)
        .unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = replace(&app, "file.txt", "old", "old", "new");
    assert_eq!(result["state"], "error", "{result}");
    assert_eq!(result["diagnostics"][0]["code"], "EDIT_OUTCOME_UNKNOWN");
    assert_eq!(result["data"]["commitStatus"], "unknown");
    assert!(result["data"]["changed"].is_null());
    let candidate = result["data"]["recovery"]["candidate"].as_str().unwrap();
    assert_eq!(
        std::fs::read_to_string(root.path().join(candidate)).unwrap(),
        "new"
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "old");
    drop(held);
}

#[test]
fn junctions_cannot_redirect_edits_outside_the_workspace() {
    use std::os::windows::process::CommandExt;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("file.txt"), "old").unwrap();
    let junction = root.path().join("junction");
    let status = std::process::Command::new("cmd")
        .creation_flags(0x0800_0000)
        .args(["/c", "mklink", "/J"])
        .arg(&junction)
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(status.status.success());
    let app = adapters::application(root.path()).unwrap();
    assert_eq!(
        replace(&app, "junction/file.txt", "old", "old", "new")["diagnostics"][0]["code"],
        "PATH"
    );
    assert_eq!(
        std::fs::read_to_string(outside.path().join("file.txt")).unwrap(),
        "old"
    );
    std::fs::remove_dir(junction).unwrap();
}

#[test]
fn concurrent_clones_cannot_both_publish_the_same_hash() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file.txt"), "old").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let gate = std::sync::Arc::new(std::sync::Barrier::new(3));
    let workers: Vec<_> = ["one", "two"]
        .into_iter()
        .map(|value| {
            let app = app.clone();
            let gate = gate.clone();
            std::thread::spawn(move || {
                gate.wait();
                replace(&app, "file.txt", "old", "old", value)
            })
        })
        .collect();
    gate.wait();
    let results: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    assert_eq!(
        results.iter().filter(|r| r["state"] == "ok").count(),
        1,
        "{results:?}"
    );
    assert!(
        results
            .iter()
            .any(|r| r["diagnostics"][0]["code"] == "STALE"
                || r["diagnostics"][0]["code"] == "EDIT_BUSY")
    );
}

#[test]
fn parserless_formats_require_explicit_opt_in() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let before = "$c: #fff;\n.box { color: $c; }\n";
    std::fs::write(root.path().join("panel.scss"), before).unwrap();
    let edit = |json: Value| {
        let args: ReplaceArgs = serde_json::from_value(json).unwrap();
        app.execute(Request::Replace(args))
    };
    let denied = edit(serde_json::json!({
        "path":"panel.scss","hash":hash(before.as_bytes()),
        "oldText":"$c: #fff;","newText":"$c: #000;"
    }));
    assert_eq!(denied["state"], "error", "{denied}");
    assert_eq!(denied["diagnostics"][0]["code"], "PARSER_UNAVAILABLE");
    assert_eq!(
        std::fs::read_to_string(root.path().join("panel.scss")).unwrap(),
        before
    );
    let allowed = edit(serde_json::json!({
        "path":"panel.scss","hash":hash(before.as_bytes()),
        "oldText":"$c: #fff;","newText":"$c: #000;","allowUnvalidatedText":true
    }));
    assert_eq!(allowed["state"], "ok", "{allowed}");
    assert_eq!(allowed["data"]["syntax"]["status"], "not_checked");
    assert_eq!(
        std::fs::read_to_string(root.path().join("panel.scss")).unwrap(),
        "$c: #000;\n.box { color: $c; }\n"
    );
}

#[test]
fn match_failure_reports_diverging_line_without_writing() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let before = "fn a() {}\nfn b() {}\nfn c() {}\n";
    std::fs::write(root.path().join("lib.rs"), before).unwrap();
    let result = app.execute(Request::Replace(ReplaceArgs {
        path: "lib.rs".into(),
        hash: hash(before.as_bytes()),
        chunks: None,
        old_text: "fn a() {}\nfn B() {}\nfn c() {}".into(),
        new_text: "fn d() {}".into(),
        allow_unvalidated_text: false,
        preview: false,
        session: None,
    }));
    assert_eq!(result["state"], "error", "{result}");
    assert_eq!(result["diagnostics"][0]["code"], "MATCH");
    assert_eq!(result["diagnostics"][0]["hint"]["firstDivergentLine"], 2);
    assert_eq!(result["diagnostics"][0]["hint"]["foundLine"], "fn b() {}");
    assert_eq!(
        std::fs::read_to_string(root.path().join("lib.rs")).unwrap(),
        before
    );
}

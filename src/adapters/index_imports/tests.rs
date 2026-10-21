use super::super::hash;
use super::*;

#[test]
fn runtime_export_memo_is_publication_local_and_sources_are_rechecked() {
    let root = tempfile::tempdir().unwrap();
    let source = "export class Service {}";
    std::fs::write(root.path().join("service.ts"), source).unwrap();
    let project = Project::new(root.path()).unwrap();
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE files(path TEXT PRIMARY KEY, content_hash TEXT, language TEXT);",
    )
    .unwrap();
    db.execute(
        "INSERT INTO files VALUES('service.ts',?1,'typescript')",
        [hash(source.as_bytes())],
    )
    .unwrap();
    let mut sources = Sources::default();
    let context = OperationContext::default();
    let kind = crate::adapters::index_semantic::RuntimeKind::Class;
    for _ in 0..3 {
        let target = sources
            .runtime_export_target(&db, &project, &context, "service.ts", "Service", kind)
            .unwrap()
            .unwrap();
        assert_eq!(target.name, "Service");
    }
    assert_eq!(sources.runtime.len(), 1);
    std::fs::write(root.path().join("service.ts"), "export class Changed {}").unwrap();
    assert!(matches!(
        sources.recheck(&project, &context),
        Err(("STALE", _))
    ));
    assert!(Sources::default().runtime.is_empty());
}
#[test]
fn verified_source_cache_enforces_file_count_budget() {
    let root = tempfile::tempdir().unwrap();
    let project = Project::new(root.path()).unwrap();
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE files(path TEXT PRIMARY KEY, content_hash TEXT, language TEXT);",
    )
    .unwrap();
    let mut sources = Sources::default();
    let context = OperationContext::default();

    for index in 0..=64 {
        let path = format!("source-{index}.ts");
        std::fs::write(root.path().join(&path), b"").unwrap();
        db.execute(
            "INSERT INTO files VALUES(?1,?2,'typescript')",
            [&path, &hash(b"")],
        )
        .unwrap();
        let result = sources.observe(&db, &project, &context, &path);
        if index < 64 {
            assert!(result.unwrap());
        } else {
            assert_eq!(result, Err(super::super::LIMIT));
        }
    }
    assert_eq!(sources.verified.len(), 64);
    assert!(
        sources
            .observe(&db, &project, &context, "source-0.ts")
            .unwrap()
    );

    let rust_path = "rust-target.rs";
    let rust_source = "fn caller() { target(); }";
    std::fs::write(root.path().join(rust_path), rust_source).unwrap();
    db.execute(
        "INSERT INTO files VALUES(?1,?2,'rust')",
        [rust_path, &hash(rust_source.as_bytes())],
    )
    .unwrap();
    assert!(matches!(
        sources.rust_target(&db, &project, &context, rust_path, "target", 1, 17),
        Ok(None)
    ));
}

#[test]
fn module_normalization_never_escapes_workspace() {
    assert_eq!(
        relative_module("src/a.ts", "../lib/b"),
        Some("lib/b".into())
    );
    for module in [
        "../../outside",
        "pkg",
        "./x\\y",
        "./x?raw",
        "./%2e%2e/x",
        "./x:y",
    ] {
        assert_eq!(relative_module("src/a.ts", module), None);
    }
}
#[test]
fn parses_named_aliases_but_defers_other_import_forms() {
    let (module, names) =
        codegraph_native::named_imports("import {run as execute, other} from './lib';").unwrap();
    assert_eq!(module, "./lib");
    assert_eq!(
        names,
        [
            ("run".into(), "execute".into()),
            ("other".into(), "other".into())
        ]
    );
    for signature in [
        "import x from './lib'",
        "import * as x from './lib'",
        "import type { X } from './lib'",
        "import {type X} from './lib'",
        "import {x} from './lib'; evil()",
        "import {x} from './l\\u0069b'",
        "import {x} from './lib' with {type:'json'}",
    ] {
        assert!(
            codegraph_native::named_imports(signature).is_none(),
            "{signature}"
        );
    }
}


    use super::*;
    #[test]
    fn rust_free_calls_position_index_matches_prefix_every_byte() {
        for source in [
            "",
            "\n",
            "\r\n",
            "\u{feff}fn é() { 😀 }\r\n\n終",
            "a\nb\r\nc\n",
        ] {
            let positions = Positions::new(source);
            for offset in 0..=source.len() + 1 {
                let expected = source.get(..offset).map(|prefix| {
                    (
                        prefix.bytes().filter(|b| *b == b'\n').count() as i64 + 1,
                        prefix.rsplit('\n').next().unwrap().encode_utf16().count() as i64,
                    )
                });
                assert_eq!(
                    positions.get(offset),
                    expected,
                    "offset {offset} in {source:?}"
                );
            }
        }
    }
    #[test]
    #[ignore = "focused phase timing; no index reads or writes"]
    fn rust_resolution_phase_probe() {
        use std::{hint::black_box, time::Instant};
        let sources = [
            (
                "src/adapters/index_store.rs",
                include_str!("../index_store.rs"),
            ),
            ("src/adapters/process.rs", include_str!("../process.rs")),
            ("src/adapters/explore.rs", include_str!("../explore.rs")),
            (
                "crates/application/src/lib.rs",
                include_str!("../../../crates/application/src/lib.rs"),
            ),
        ];
        let mut extract = 0;
        let mut syntax = 0;
        let mut positions = 0;
        let mut indexed_positions = 0;
        let context = OperationContext::default();
        for _ in 0..10 {
            for (path, source) in sources {
                let started = Instant::now();
                black_box(
                    super::super::super::index_extract::extract(path, source, &context).unwrap(),
                );
                extract += started.elapsed().as_micros();
                let started = Instant::now();
                black_box(analyze(source, &context).unwrap());
                syntax += started.elapsed().as_micros();
                let started = Instant::now();
                for (offset, _) in source.char_indices().step_by(97) {
                    let prefix = &source[..offset];
                    black_box((
                        prefix.bytes().filter(|b| *b == b'\n').count() + 1,
                        prefix.rsplit('\n').next().unwrap().encode_utf16().count(),
                    ));
                }
                positions += started.elapsed().as_micros();
                let started = Instant::now();
                let index = Positions::new(source);
                for (offset, _) in source.char_indices().step_by(97) {
                    black_box(index.get(offset).unwrap());
                }
                indexed_positions += started.elapsed().as_micros();
            }
        }
        println!(
            "{}",
            serde_json::json!({"probe":"rust-resolution-phases","rounds":10,"filesPerRound":4,"debugAssertions":cfg!(debug_assertions),"extractionMs":extract as f64/1000.0,"rustProofMs":syntax as f64/1000.0,"prefixPositionsMs":positions as f64/1000.0,"indexedPositionsMs":indexed_positions as f64/1000.0,"includesDatabase":false})
        );
    }
    #[cfg(windows)]
    #[test]
    fn rust_free_calls_real_sync_workspace_binding() {
        use super::super::{initialize, sync_file};
        let root = tempfile::tempdir().unwrap();
        let context = OperationContext::default();
        initialize(root.path(), &context).unwrap();
        std::fs::create_dir_all(root.path().join("src/adapters")).unwrap();
        for (path, source) in [
            ("src/lib.rs", include_str!("../../lib.rs")),
            ("src/adapters/mod.rs", include_str!("../mod.rs")),
            (
                "src/adapters/index_store.rs",
                include_str!("../index_store.rs"),
            ),
            ("src/adapters/explore.rs", include_str!("../explore.rs")),
        ] {
            std::fs::write(root.path().join(path), source).unwrap();
            sync_file(root.path(), path, &context).unwrap();
        }
        let db = rusqlite::Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
        let count: i64 = db.query_row("SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND s.name='synchronized' AND t.name='sync_workspace_all' AND json_extract(e.metadata,'$.resolvedBy')='ra-module-subset-v1'", [], |r| r.get(0)).unwrap();
        assert_eq!(
            count, 1,
            "the actual harness call must resolve, not just a synthetic fixture"
        );
    }
    #[cfg(windows)]
    #[test]
    fn rust_free_calls_publication_proves_sibling_and_rejects_homonyms() {
        use super::super::{initialize, sync_file};
        let root = tempfile::tempdir().unwrap();
        let context = OperationContext::default();
        initialize(root.path(), &context).unwrap();
        std::fs::create_dir_all(root.path().join("src/adapters")).unwrap();
        let files = [
            ("src/lib.rs", "mod adapters;"),
            ("src/adapters/mod.rs", "mod explore; mod index_store;"),
            (
                "src/adapters/index_store.rs",
                "/// Indexed target\npub(super) fn sync_workspace_all() {}",
            ),
            (
                "src/adapters/explore.rs",
                "fn collect() {}\nfn execute() {}\nfn run() {}\nfn good() {}\nfn synchronized() { x.collect(); stmt.execute(); let run = || {}; run(); good(); super::index_store::sync_workspace_all(); }\n",
            ),
        ];
        for (path, text) in files {
            std::fs::write(root.path().join(path), text).unwrap();
            sync_file(root.path(), path, &context).unwrap();
        }
        let db = rusqlite::Connection::open(root.path().join(".codegraph/codegraph.db")).unwrap();
        let mut stmt = db.prepare("SELECT t.name FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' ORDER BY t.name").unwrap();
        let targets = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(targets, ["good", "sync_workspace_all"]);
        let pending: i64 = db
            .query_row(
                "SELECT count(*) FROM unresolved_refs WHERE reference_kind='calls'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(pending >= 3);
        drop(stmt);
        let sibling_calls = || {
            db.query_row("SELECT count(*) FROM edges e JOIN nodes t ON t.id=e.target WHERE e.kind='calls' AND t.name='sync_workspace_all'", [], |r| r.get::<_, i64>(0)).unwrap()
        };
        std::fs::write(
            root.path().join("src/adapters/mod.rs"),
            "mod explore; #[path=\"other.rs\"] mod index_store;",
        )
        .unwrap();
        sync_file(root.path(), "src/adapters/mod.rs", &context).unwrap();
        assert_eq!(
            sibling_calls(),
            0,
            "changed module proof must invalidate the old edge"
        );
        std::fs::write(
            root.path().join("src/adapters/mod.rs"),
            "mod explore; mod index_store;",
        )
        .unwrap();
        sync_file(root.path(), "src/adapters/mod.rs", &context).unwrap();
        assert_eq!(
            sibling_calls(),
            1,
            "pending qualified calls must retry after module changes"
        );
        std::fs::create_dir(root.path().join("src/adapters/index_store")).unwrap();
        std::fs::write(
            root.path().join("src/adapters/index_store/mod.rs"),
            "pub fn sync_workspace_all() {}",
        )
        .unwrap();
        sync_file(root.path(), "src/adapters/index_store/mod.rs", &context).unwrap();
        assert_eq!(sibling_calls(), 0, "ambiguous module files must not bind");
    }
    #[test]
    fn rust_free_calls_reject_methods_and_shadowed_closures() {
        let source = "fn collect() {}\nfn run() {}\nfn good() {}\nfn caller() { x.collect(); let run = || {}; run(); good(); super::store::sync_workspace_all(); }\n";
        let facts = analyze(source, &OperationContext::default()).unwrap();
        let names: Vec<_> = facts.calls.keys().map(|k| k.0.as_str()).collect();
        assert_eq!(names, vec!["good", "super::store::sync_workspace_all"]);
    }
    #[test]
    fn rust_free_calls_reject_parameters_patterns_and_imports() {
        for body in [
            "fn f() { run().run(); }",
            "fn f(run: fn()) { run(); }",
            "fn f() { match x { Some(run) => run(), _ => () } }",
            "use other::run; fn f() { run(); }",
        ] {
            let facts = analyze(
                &format!("fn run() {{}} {body}"),
                &OperationContext::default(),
            )
            .unwrap();
            assert!(facts.calls.is_empty());
        }
    }
    #[test]
    fn rust_free_calls_use_utf16_columns_and_preserve_bom_crlf() {
        let source = "\u{feff}fn good() {}\r\nfn caller() { let s = \"😀\"; good(); }\r\n";
        let facts = analyze(source, &OperationContext::default()).unwrap();
        let offset = source.rfind("good()").unwrap();
        let prefix = &source[..offset];
        let col = prefix.rsplit('\n').next().unwrap().encode_utf16().count() as i64;
        assert!(facts.call("good", 2, col).is_some());
    }

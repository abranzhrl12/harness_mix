    use super::*;

    #[test]
    fn archive_status_tracks_typed_replacements_and_scope_without_writing() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE nodes(id TEXT PRIMARY KEY,file_path TEXT,qualified_name TEXT,kind TEXT);
            CREATE TABLE edges(source TEXT,target TEXT,kind TEXT,metadata TEXT);
            CREATE TABLE native_invalidated_edges(origin_path TEXT,snapshot_hash TEXT,snapshot TEXT,reason TEXT);
            INSERT INTO nodes VALUES('trait','port.rs','Port::run','method'),('implementation','impl.rs','Impl::run','method'),('unrelated','other.rs','unrelated','function');").unwrap();
        let snapshot = json!({"source":"trait","target":"implementation","kind":"calls",
            "sourcePath":"port.rs","targetPath":"impl.rs","sourceQualifiedName":"Port::run","targetQualifiedName":"Impl::run","sourceKind":"method","targetKind":"method",
            "metadata":json!({"synthesizedBy":"interface-impl"}).to_string()});
        db.execute(
            "INSERT INTO native_invalidated_edges VALUES('port.rs','snapshot',?1,'legacy')",
            [snapshot.to_string()],
        )
        .unwrap();
        db.execute_batch("PRAGMA query_only=ON").unwrap();
        assert_eq!(coverage(&db, None, None).unwrap().pending, 1);
        assert_eq!(
            coverage(&db, Some("unrelated.rs"), None).unwrap(),
            Coverage::default()
        );
        assert_eq!(
            coverage(
                &db,
                None,
                Some(&std::collections::BTreeSet::from(["unrelated".to_owned()]))
            )
            .unwrap(),
            Coverage::default()
        );
        db.execute_batch("PRAGMA query_only=OFF; INSERT INTO edges VALUES('trait','implementation','calls',NULL)").unwrap();
        // A call edge is not proof of a trait implementation.
        assert_eq!(coverage(&db, None, None).unwrap().pending, 1);
        db.execute_batch("INSERT INTO edges VALUES('trait','implementation','implements',NULL)")
            .unwrap();
        assert_eq!(
            coverage(&db, Some("impl.rs"), None).unwrap(),
            Coverage {
                superseded: 1,
                ..Coverage::default()
            }
        );
        db.execute_batch("DELETE FROM nodes WHERE id='implementation'")
            .unwrap();
        assert_eq!(
            coverage(&db, None, None).unwrap(),
            Coverage {
                obsolete: 1,
                ..Coverage::default()
            }
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM native_invalidated_edges", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            1
        );
    }

    #[test]
    fn corrupt_archive_metadata_is_unknown_not_an_error_or_success() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE nodes(id TEXT,file_path TEXT,qualified_name TEXT,kind TEXT); CREATE TABLE edges(source TEXT,target TEXT,kind TEXT,metadata TEXT);
            CREATE TABLE native_invalidated_edges(origin_path TEXT,snapshot_hash TEXT,snapshot TEXT,reason TEXT);
            INSERT INTO native_invalidated_edges VALUES('a.rs','one','invalid JSON','legacy');").unwrap();
        assert_eq!(coverage(&db, None, None).unwrap().pending, 1);
        db.execute_batch("INSERT INTO nodes VALUES('a','a.rs','a','function'),('b','b.rs','b','function'); INSERT INTO edges VALUES('a','b','calls',NULL)").unwrap();
        db.execute(
            "INSERT INTO native_invalidated_edges VALUES('a.rs','two',?1,'legacy')",
            [
                json!({"source":"a","target":"b","kind":"calls","metadata":"invalid JSON"})
                    .to_string(),
            ],
        )
        .unwrap();
        assert_eq!(coverage(&db, None, None).unwrap().pending, 2);
    }

    #[test]
    fn derived_edge_invalidation_is_transactional_and_preserves_unrelated_edges() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE nodes(id TEXT PRIMARY KEY,file_path TEXT,kind TEXT,qualified_name TEXT);
            CREATE TABLE files(path TEXT PRIMARY KEY,content_hash TEXT);
            CREATE TABLE edges(id INTEGER PRIMARY KEY,source TEXT,target TEXT,kind TEXT,line INTEGER,col INTEGER,metadata TEXT,provenance TEXT);
            INSERT INTO nodes(id,file_path,kind) VALUES('a','a.rs','function'),('b','b.rs','function'),('c','c.rs','function');
            INSERT INTO files VALUES('a.rs','hash-a'),('b.rs','hash-b');
            INSERT INTO edges VALUES(1,'a','b','calls',1,0,'{\"synthesizedBy\":\"framework\"}','heuristic'),
                (2,'b','c','calls',2,0,'{\"synthesizedBy\":\"framework\"}',NULL),
                (3,'a','b','references',3,0,NULL,NULL);").unwrap();
        {
            let tx = db.transaction().unwrap();
            invalidate(&tx, "a.rs", &OperationContext::default()).unwrap();
            assert_eq!(coverage(&tx, Some("a.rs"), None).unwrap().pending, 1);
            // A failure after capture must roll back archive creation/deletion,
            // not just fail schema validation before the publication starts.
            assert!(
                tx.execute(
                    "INSERT INTO nodes(id,file_path,kind) VALUES('a','other.rs','function')",
                    []
                )
                .is_err()
            );
            tx.rollback().unwrap();
        }
        assert_eq!(coverage(&db, None, None).unwrap(), Coverage::default());
        assert_eq!(
            db.query_row("SELECT count(*) FROM edges", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            3
        );
        let tx = db.transaction().unwrap();
        invalidate(&tx, "a.rs", &OperationContext::default()).unwrap();
        tx.commit().unwrap();
        assert_eq!(coverage(&db, Some("a.rs"), None).unwrap().pending, 1);
        assert_eq!(coverage(&db, Some("c.rs"), None).unwrap().pending, 0);
        assert_eq!(
            db.query_row("SELECT count(*) FROM edges WHERE id IN (2,3)", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
    }

    use super::*;
    use rusqlite::Connection;

    fn dependency_db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE nodes(id TEXT PRIMARY KEY,file_path TEXT,language TEXT);
             CREATE TABLE edges(source TEXT,target TEXT,metadata TEXT);
             CREATE TABLE unresolved_refs(file_path TEXT,language TEXT,reference_kind TEXT,status TEXT);
             INSERT INTO nodes VALUES('resolved','Resolved.cs','csharp');
             INSERT INTO nodes VALUES('wrong-language','Other.js','javascript');
             INSERT INTO edges VALUES('resolved','sql-local:proc','{\"resolvedBy\":\"local-sql-literal-v1\",\"refKind\":\"stored_procedure_call\"}');
             INSERT INTO edges VALUES('wrong-language','sql-local:proc','{\"resolvedBy\":\"local-sql-literal-v1\",\"refKind\":\"stored_procedure_call\"}');
             INSERT INTO unresolved_refs VALUES('Pending.cs','csharp','stored_procedure_call','pending');
             INSERT INTO unresolved_refs VALUES('Failed.cs','csharp','stored_procedure_call','failed');",
        )
        .unwrap();
        db
    }

    #[test]
    fn sql_dependency_collection_includes_resolved_and_pending_callers_only() {
        let db = dependency_db();
        let paths = NativeChangedPaths::sql_callers(&db).unwrap();
        assert_eq!(paths, ["Failed.cs", "Pending.cs", "Resolved.cs"]);
    }

    #[test]
    fn sql_dependency_collection_enforces_limit_before_resync() {
        let db = dependency_db();
        for index in 0..257 {
            db.execute(
                "INSERT INTO unresolved_refs VALUES(?1,'csharp','stored_procedure_call','pending')",
                [format!("Caller{index:03}.cs")],
            )
            .unwrap();
        }
        assert_eq!(
            NativeChangedPaths::sql_callers(&db).unwrap_err().0,
            "SYNC_LIMIT"
        );
    }

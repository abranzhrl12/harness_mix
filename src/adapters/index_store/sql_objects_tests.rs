    use super::*;
    #[test]
    fn sql_literals_and_comments_do_not_create_objects_or_dependencies() {
        let source = "-- CREATE TABLE dbo.Fake(x int)\nCREATE TABLE [dbo].[Orders](id int);\nGO\nCREATE OR ALTER PROC [dbo].[Find] AS\nSELECT 'FROM dbo.Fake' FROM [dbo].[Orders]; /* JOIN dbo.Fake */\nGO";
        let objects = parse("schema.sql", source, "hash");
        assert_eq!(objects.len(), 2);
        assert_eq!(objects[1].name, "dbo.Find");
        assert_eq!(objects[1].dependencies, vec![("dbo.Orders".to_owned(), 5)]);
    }

    #[test]
    fn cte_and_table_valued_function_homonyms_are_not_bound_to_tables() {
        let objects = parse(
            "schema.sql",
            "CREATE PROC Find AS\nWITH Orders AS (SELECT 1 AS Id) SELECT * FROM Orders;\nSELECT * FROM dbo.Orders(1);\nGO",
            "hash",
        );
        assert!(objects[0].dependencies.is_empty());
    }

    #[test]
    fn update_alias_does_not_bind_an_unrelated_table_with_that_name() {
        let objects = parse(
            "schema.sql",
            "CREATE PROC Find AS UPDATE Alias SET Id=1 FROM dbo.Orders Alias;\nGO",
            "hash",
        );
        assert_eq!(objects[0].dependencies, vec![("dbo.Orders".to_owned(), 1)]);
    }

    #[test]
    fn removing_last_declaration_then_sql_file_cleans_its_tracking_row() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(include_str!("../../../vendor/codegraph-native/schema.sql"))
            .unwrap();
        let context = OperationContext::default();
        let objects = parse("schema.sql", "CREATE TABLE dbo.Orders(id int);", "first");
        let mut files = Files::from([("schema.sql".to_owned(), ("first".to_owned(), 32))]);
        publish(&db, &objects, &files, &context).unwrap();
        files.insert("schema.sql".to_owned(), ("empty".to_owned(), 0));
        publish(&db, &[], &files, &context).unwrap();
        let digest: String = db
            .query_row(
                "SELECT content_hash FROM files WHERE path='schema.sql'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(digest, "empty");
        publish(&db, &[], &Files::new(), &context).unwrap();
        assert_eq!(
            db.query_row("SELECT count(*) FROM files", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM nodes", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

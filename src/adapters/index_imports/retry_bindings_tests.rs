use super::*;

#[test]
fn catalog_keeps_import_aliases_and_local_candidate_names_but_not_arbitrary_calls() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE nodes(file_path TEXT,kind TEXT,name TEXT,qualified_name TEXT,signature TEXT,start_line INTEGER,id TEXT,language TEXT);").unwrap();
    db.execute(
        "INSERT INTO nodes VALUES('app.js','import','./dep','./dep',?1,1,'i','javascript')",
        ["import { run as imported } from './dep';"],
    )
    .unwrap();
    db.execute(
        "INSERT INTO nodes VALUES('app.js','function','local','scope.local',NULL,2,'f','javascript')",
        [],
    )
    .unwrap();
    let text = catalog_inner(&db).unwrap().unwrap();
    let pairs: BTreeSet<(String, String)> = serde_json::from_str(&text).unwrap();
    assert_eq!(
        pairs,
        BTreeSet::from([
            ("app.js".into(), "imported".into()),
            ("app.js".into(), "local".into()),
            ("app.js".into(), "scope.local".into())
        ])
    );
}

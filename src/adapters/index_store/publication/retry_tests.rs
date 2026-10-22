use super::*;
use rusqlite::Connection;

#[test]
fn private_script_names_only_retry_their_own_file_but_modules_remain_conservative() {
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE nodes(file_path TEXT,kind TEXT,name TEXT,qualified_name TEXT,visibility TEXT,is_exported INTEGER);
        CREATE TABLE unresolved_refs(id INTEGER PRIMARY KEY,reference_name TEXT,file_path TEXT,language TEXT,reference_kind TEXT,status TEXT);
        INSERT INTO nodes VALUES('changed.js','function','hidden','hidden',NULL,0),('changed.js','function','visible','visible',NULL,1),('importer.js','import','alias','alias',NULL,0);
        INSERT INTO unresolved_refs VALUES(1,'hidden','other.js','javascript','calls','pending'),(2,'hidden','changed.js','javascript','calls','pending'),(3,'visible','other.js','javascript','calls','pending'),(4,'hidden','importer.js','javascript','calls','pending'),(5,'hidden','other.js','javascript','angular_injects','pending'),(6,'visible','other.js','javascript','calls','failed');").unwrap();
    let tx = db.transaction().unwrap();
    let mut selected = pending(&tx, "changed.js", "javascript", false, "[]", None).unwrap();
    selected.sort_unstable();
    assert_eq!(selected, vec![2, 3, 4, 5]);
    // C#/Rust have language-specific binding/visibility rules; do not apply
    // the classic-script shortcut to those languages.
    let mut selected = pending(&tx, "changed.js", "rust", false, "[]", None).unwrap();
    selected.sort_unstable();
    assert_eq!(selected, vec![1, 2, 3, 4, 5]);
    let mut selected = pending(
        &tx,
        "changed.js",
        "javascript",
        false,
        r#"["visible"]"#,
        None,
    )
    .unwrap();
    selected.sort_unstable();
    assert_eq!(selected, vec![2, 4, 5]);
    let mut selected = pending(
        &tx,
        "changed.js",
        "javascript",
        false,
        "[]",
        Some(r#"[["importer.js","another_binding"]]"#),
    )
    .unwrap();
    selected.sort_unstable();
    assert_eq!(selected, vec![2, 3, 5]);
}

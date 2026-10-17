use rusqlite::{Connection, params};
use std::path::Path;

pub fn fixture(root: &Path) -> Connection {
    std::fs::create_dir_all(root.join(".codegraph")).unwrap();
    let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    // Columns consumed from the installed CodeGraph schema v11. Extra columns,
    // FTS tables and indexer internals are deliberately not required by readers.
    db.execute_batch("CREATE TABLE schema_versions(version INTEGER PRIMARY KEY); INSERT INTO schema_versions VALUES(11);
        CREATE TABLE nodes(id TEXT PRIMARY KEY,name TEXT,kind TEXT,file_path TEXT,start_line INTEGER);
        CREATE INDEX idx_nodes_name ON nodes(name);
        CREATE TABLE edges(id INTEGER PRIMARY KEY,source TEXT,target TEXT,kind TEXT);
        CREATE INDEX idx_edges_source ON edges(source); CREATE INDEX idx_edges_target ON edges(target);
        CREATE TABLE files(path TEXT PRIMARY KEY,content_hash TEXT,errors TEXT);").unwrap();
    std::fs::write(
        root.join("lib.rs"),
        "pub fn target() { callee(); }\npub fn caller() { target(); }\npub fn callee() {}\n",
    )
    .unwrap();
    for (id, name, line) in [("t", "target", 1), ("c", "caller", 2), ("d", "callee", 3)] {
        db.execute(
            "INSERT INTO nodes VALUES(?1,?2,'function','lib.rs',?3)",
            params![id, name, line],
        )
        .unwrap();
    }
    db.execute_batch("INSERT INTO edges VALUES(1,'c','t','calls'); INSERT INTO edges VALUES(2,'t','d','calls'); INSERT INTO edges VALUES(3,'c','t','contains');").unwrap();
    db.execute(
        "INSERT INTO files VALUES('lib.rs',?1,NULL)",
        [harness_domain::hash(
            &std::fs::read(root.join("lib.rs")).unwrap(),
        )],
    )
    .unwrap();
    db
}

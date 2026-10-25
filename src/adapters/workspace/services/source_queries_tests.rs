use super::*;
use crate::adapters::{Project, external_scopes::Registry, project::hash};
use std::{fs, sync::{Arc, Mutex}};

#[test]
fn read_line_numbers_are_display_only_and_respect_the_selected_eof_line() {
    let dir = tempfile::tempdir().unwrap();
    let source = b"first\r\nsecond\r\n";
    fs::write(dir.path().join("example.txt"), source).unwrap();
    let service = SourceQueryService::new(
        Project::new(dir.path()).unwrap(),
        Arc::new(Mutex::new(Registry::default())),
        Arc::new(Mutex::new(())),
    );
    let context = OperationContext::default();
    let read_args = |line_numbers| ReadArgs {
        external_scope: None,
        path: "example.txt".into(),
        start: 2,
        end: 3,
        compact: false,
        line_numbers,
        session: None,
    };

    let numbered = service.read(&read_args(true), &context).unwrap();
    assert_eq!(numbered["text"], "second\r\n");
    assert_eq!(numbered["numberedText"], "2 | second\r\n3 | ");
    assert_eq!(numbered["hash"], hash(source));

    let plain = service.read(&read_args(false), &context).unwrap();
    assert_eq!(plain["text"], numbered["text"]);
    assert_eq!(plain["hash"], numbered["hash"]);
    assert!(plain.get("numberedText").is_none());
}

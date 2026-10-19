#![cfg(windows)]

use agent_harness::adapters;
use harness_domain::{contracts::Request, hash};
use serde_json::{Value, json};

fn execute(app: &adapters::NativeApplication, name: &str, args: Value) -> Value {
    let request = match name {
        "write" => Request::Write(serde_json::from_value(args).unwrap()),
        _ => Request::Replace(serde_json::from_value(args).unwrap()),
    };
    let result = app.execute(request);
    assert_eq!(result["state"], "ok", "{result}");
    result
}

#[test]
fn razor_creation_replace_preview_and_overwrite_hash_the_encoded_bytes() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    let path = root.path().join("view.cshtml");
    let created = execute(
        &app,
        "write",
        json!({"path":"view.cshtml","content":"<p>Vehículo</p>"}),
    );
    let first = "\u{feff}<p>Vehículo</p>";
    assert_eq!(std::fs::read(&path).unwrap(), first.as_bytes());
    assert_eq!(created["data"]["hash"], hash(first.as_bytes()));
    let preview = execute(
        &app,
        "replace",
        json!({"path":"view.cshtml","hash":hash(first.as_bytes()),
        "oldText":"Vehículo","newText":"TELÉFONO","preview":true}),
    );
    let next = "\u{feff}<p>TELÉFONO</p>";
    assert_eq!(preview["data"]["candidateHash"], hash(next.as_bytes()));
    assert_eq!(std::fs::read(&path).unwrap(), first.as_bytes());
    let replaced = execute(
        &app,
        "replace",
        json!({"path":"view.cshtml","hash":hash(first.as_bytes()),
        "oldText":"Vehículo","newText":"TELÉFONO"}),
    );
    assert_eq!(std::fs::read(&path).unwrap(), next.as_bytes());
    assert_eq!(replaced["data"]["hash"], hash(next.as_bytes()));
    let overwritten = execute(
        &app,
        "write",
        json!({"path":"view.cshtml","content":"<p>Árbol</p>","overwrite":true}),
    );
    let last = "\u{feff}<p>Árbol</p>";
    assert_eq!(std::fs::read(&path).unwrap(), last.as_bytes());
    assert_eq!(overwritten["data"]["hash"], hash(last.as_bytes()));
    assert_eq!(overwritten["data"]["verified"], true);
}

#[test]
fn explicit_bom_overrides_are_idempotent_and_replace_can_remove_it() {
    let root = tempfile::tempdir().unwrap();
    let app = adapters::application(root.path()).unwrap();
    for (path, content, bom, expected) in [
        ("plain.cshtml", "<p>á</p>", false, "<p>á</p>"),
        ("marked.txt", "\u{feff}á", true, "\u{feff}á"),
        ("clean.txt", "\u{feff}á", false, "á"),
    ] {
        let request = json!({"path":path,"content":content,"bom":bom});
        execute(&app, "write", request.clone());
        let repeated = execute(&app, "write", request);
        assert_eq!(repeated["data"]["changed"], false);
        assert_eq!(
            std::fs::read(root.path().join(path)).unwrap(),
            expected.as_bytes()
        );
    }
    let removed = execute(
        &app,
        "replace",
        json!({"path":"marked.txt","oldText":"á","newText":"é","bom":false}),
    );
    assert_eq!(
        std::fs::read(root.path().join("marked.txt")).unwrap(),
        "é".as_bytes()
    );
    assert_eq!(removed["data"]["hash"], hash("é".as_bytes()));
}

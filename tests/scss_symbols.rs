use agent_harness::adapters;
use harness_domain::{
    contracts::{OutlineArgs, ReadSymbolArgs, ReplaceArgs, Request},
    hash,
};
use serde_json::Value;

fn outline(app: &agent_harness::adapters::NativeApplication, path: &str) -> Value {
    app.execute(Request::Outline(OutlineArgs {
        path: path.into(),
        query: None,
        max_chars: Some(16000),
        filter_kind: None,
        view: None,
        session: None,
        external_scope: None,
    }))
}

fn read(
    app: &agent_harness::adapters::NativeApplication,
    path: &str,
    name: &str,
    id: Option<&str>,
) -> Value {
    app.execute(Request::ReadSymbol(ReadSymbolArgs {
        path: path.into(),
        symbol: name.into(),
        container: None,
        symbol_id: id.map(Into::into),
        view: None,
        session: None,
        external_scope: None,
    }))
}

fn replace(
    app: &agent_harness::adapters::NativeApplication,
    path: &str,
    old_text: &str,
    new_text: &str,
) -> Value {
    app.execute(Request::Replace(ReplaceArgs {
        path: path.into(),
        bom: None,
        hash: String::new(),
        line: None,
        chunks: None,
        old_text: old_text.into(),
        new_text: new_text.into(),
        allow_unvalidated_text: false,
        preview: false,
        session: None,
    }))
}

const STYLES: &str = r#"@use "sass:math";
@use "../shared/tokens" as tokens;

$brand: #1976d2;
$radius: 4px;

@mixin card($padding: 8px) {
  padding: $padding;
  border-radius: $radius;
}

@function double($value) {
  @return $value * 2;
}

.card {
  @include card(12px);
  color: $brand;
  width: math.div(100%, 3);

  &__title {
    font-weight: 600;
  }

  &:hover {
    color: tokens.$hover;
  }
}

@media (width > 600px) {
  .card {
    padding: 24px;
  }
}
"#;

#[test]
fn scss_outline_reports_rules_variables_mixins_and_functions() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("card.scss"), STYLES).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let out = outline(&app, "card.scss");
    assert_eq!(out["state"], "ok", "{out}");
    let items = out["data"]["items"].as_array().unwrap();
    let names = items
        .iter()
        .map(|item| item["name"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();

    assert!(names.contains(&"$brand"), "{names:?}");
    assert!(names.contains(&"$radius"), "{names:?}");
    assert!(names.contains(&"card"), "{names:?}");
    assert!(names.contains(&"double"), "{names:?}");
    assert!(names.contains(&"@media (width > 600px)"), "{names:?}");
    assert!(
        names.iter().any(|name| name.contains("&__title")),
        "{names:?}"
    );
    assert!(names.contains(&"card"), "{names:?}");

    let mixin = items
        .iter()
        .find(|item| item["kind"] == "scss-mixin")
        .expect("mixin symbol");
    assert_eq!(mixin["name"], "card");
    let variable = items
        .iter()
        .find(|item| item["kind"] == "scss-variable")
        .expect("variable symbol");
    assert_eq!(variable["name"], "$brand");
}

#[test]
fn scss_nested_rules_keep_their_parent_container() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("card.scss"), STYLES).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let out = outline(&app, "card.scss");
    let items = out["data"]["items"].as_array().unwrap();
    let title = items
        .iter()
        .find(|item| item["name"].as_str().unwrap_or_default().contains("&__title"))
        .expect("nested rule");
    assert_eq!(title["container"], ".card", "{title}");
    let nested = items
        .iter()
        .filter(|item| item["name"].as_str().unwrap_or_default() == ".card")
        .find(|item| {
            item["container"]
                .as_str()
                .unwrap_or_default()
                .contains("@media")
        })
        .expect("rule inside @media");
    assert!(
        nested["container"]
            .as_str()
            .unwrap_or_default()
            .contains("@media (width > 600px)"),
        "{nested}"
    );
}

#[test]
fn scss_symbol_read_returns_the_exact_source() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("card.scss"), STYLES).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = read(&app, "card.scss", "$brand", None);
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["text"], "$brand: #1976d2");
    assert_eq!(result["data"]["hash"], hash(STYLES.as_bytes()));
}

#[test]
fn scss_edits_need_no_unvalidated_text_opt_in() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("card.scss");
    std::fs::write(&path, "$brand: #1976d2;\n.card { color: $brand; }\n").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = replace(&app, "card.scss", "#1976d2", "#0d47a1");
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["changed"], true, "{result}");
    assert_eq!(result["data"]["syntax"]["language"], "scss", "{result}");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "$brand: #0d47a1;\n.card { color: $brand; }\n"
    );
}

#[test]
fn broken_scss_is_reported_with_a_position_and_keeps_the_original() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("card.scss");
    let before = "$brand: #1976d2;\n.card { color: $brand;\n";
    std::fs::write(&path, before).unwrap();
    let app = adapters::application(root.path()).unwrap();
    let result = replace(&app, "card.scss", "#1976d2", "#0d47a1");
    assert_eq!(result["state"], "error", "{result}");
    assert_eq!(result["diagnostics"][0]["code"], "SYNTAX_ERROR", "{result}");
    let message = result["diagnostics"][0]["message"]
        .as_str()
        .unwrap_or_default();
    assert!(message.contains("line"), "{message}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn scss_outline_reports_a_syntax_error_for_broken_sources() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("bad.scss"), ".card { color: red;\n").unwrap();
    let app = adapters::application(root.path()).unwrap();
    let out = outline(&app, "bad.scss");
    assert_eq!(out["state"], "error", "{out}");
    assert_eq!(out["diagnostics"][0]["code"], "SYNTAX_ERROR", "{out}");
}

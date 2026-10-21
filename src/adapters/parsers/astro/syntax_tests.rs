use super::*;

#[test]
fn frontmatter_extraction_and_syntax_validation() {
    let ctx = OperationContext::default();
    let valid = r#"---
import Header from './Header.astro';
interface Props {
  title: string;
}
const { title } = Astro.props;
---
<Header title={title} />
<main>
  <h1>{title}</h1>
</main>
<style>
  main { padding: 1rem; }
</style>
"#;
    let res = validate("test.astro", valid, &ctx).unwrap();
    assert_eq!(res["status"], "valid");
    assert_eq!(res["frontmatterChecked"], true);

    let digest = hash(valid.as_bytes());
    let symbols = collect_symbols("test.astro", valid, &digest, &ctx).unwrap();
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Props" && s.kind == "interface_declaration")
    );
    assert!(symbols.iter().any(|s| s.name == "title"));
    assert!(symbols.iter().any(|s| s.kind == "astro_import"));
}

#[test]
fn rejects_unterminated_frontmatter_or_script() {
    let ctx = OperationContext::default();
    let unclosed = "--- \nconst a = 1;\n<div>hello</div>";
    assert_eq!(
        validate("bad.astro", unclosed, &ctx).unwrap_err().0,
        "SYNTAX_ERROR"
    );

    let bad_script = "---\nconst a = 1;\n---\n<script>\nconst x: = ;\n</script>";
    assert!(validate("bad_js.astro", bad_script, &ctx).is_err());
}

#[test]
fn component_without_frontmatter_is_valid() {
    let ctx = OperationContext::default();
    let html_only = "<main><h1>Hello World</h1></main>";
    let res = validate("simple.astro", html_only, &ctx).unwrap();
    assert_eq!(res["status"], "valid");
    assert_eq!(res["frontmatterChecked"], false);
}

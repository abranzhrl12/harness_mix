use super::*;

#[test]
fn validates_realistic_section_and_reports_basic_scope() {
    let source = r#"@model IEnumerable<Example>
@{ var value = "<not-a-tag>"; }
@section Scripts { <script>if (1 < 2) { alert("}"); }</script> }
<style>@@media screen { .x { color: red; } }</style><div><input disabled></div>
@if (a<b) { <p>@(a<b ? "yes" : "no")</p> }"#;
    let result = validate("x.cshtml", source, &OperationContext::default()).unwrap();
    assert_eq!(result["validation"], "basic_razor");
    assert_eq!(result["errors"], 0);
}

#[test]
fn catches_media_with_line_and_compiler_hint_but_skips_razor_comments() {
    let result = diagnose(
        "@* @media ignored *@\n<style>@media screen {}</style>",
        &OperationContext::default(),
    )
    .unwrap();
    assert_eq!(result["errors"], 1);
    assert_eq!(result["diagnostics"][0]["line"], 2);
    assert!(
        result["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("CS0103")
    );
}

#[test]
fn catches_sections_and_unbalanced_html() {
    for (source, code) in [
        ("@section Scripts <div></div>", "RAZOR_SECTION"),
        ("@section Scripts { <div></div>", "RAZOR_SECTION_UNCLOSED"),
        ("@section A {} @section A {}", "RAZOR_SECTION_DUPLICATE"),
        ("<div><span></div>", "RAZOR_HTML_UNBALANCED"),
        ("<main>", "RAZOR_HTML_UNCLOSED"),
    ] {
        let result = diagnose(source, &OperationContext::default()).unwrap();
        assert!(
            result["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["code"] == code),
            "{source}: {result}"
        );
        assert!(validate("x.cshtml", source, &OperationContext::default()).is_err());
    }
}

#[test]
fn escaped_transitions_comments_unicode_and_raw_text_do_not_create_tags() {
    let source = "@* <bad> *@<!-- <bad> --><div title='a > b'>á 😀</div><script>const s = '<fake>'; if (a < b) {}</script><style>@@media screen {}</style>";
    assert!(validate("x.cshtml", source, &OperationContext::default()).is_ok());
}

#[test]
fn editing_and_csharp_entry_points_use_razor_checks() {
    let ctx = OperationContext::default();
    let valid = "<style>@@media screen {}</style>";
    assert_eq!(
        crate::adapters::edit::syntax("x.CSHTML", valid, &ctx).unwrap()["validation"],
        "basic_razor"
    );
    assert_eq!(
        crate::adapters::parsers::csharp::syntax::validate_syntax("x.CSHTML", valid, &ctx).unwrap()
            ["language"],
        "razor"
    );
    assert_eq!(
        crate::adapters::edit::syntax("x.cshtml", "<style>@media screen {}</style>", &ctx)
            .unwrap_err()
            .0,
        "RAZOR_UNESCAPED_MEDIA"
    );
}

#[test]
fn html_attributes_check_razor_transitions_and_nested_csharp_quotes() {
    let ctx = OperationContext::default();
    for source in [
        r#"<input value=@Model.Email pattern=@@ />"#,
        r#"<input value=@(Model.Email) pattern=^[^@@]+$>"#,
        r#"<input value=user@example.com/>"#,
        r#"<input pattern="^[^\s@@]+@@[^\s@@]+$" />"#,
        r#"<input value="@Model.Email" pattern="@Model.Pattern" />"#,
        r#"<input value="@(Model.Email ?? "a@b.com")" />"#,
        r#"<input value='@Html.Raw("hello")' />"#,
        r#"<input value="@Model.Items[0].Email" />"#,
        r#"<a href="mailto:support@example.com" data-email="user@example.com">Mail</a>"#,
        r#"<input value="prefix @Model.Email suffix" />"#,
        r#"<input value="@* ignore @ here *@OK" />"#,
        r#"<input value="path\" />"#,
    ] {
        assert!(validate("x.cshtml", source, &ctx).is_ok(), "{source}");
    }
    for (source, code) in [
        (r#"<input pattern=^[^@]+$>"#, "RAZOR_ATTRIBUTE_TRANSITION"),
        (
            r#"<input pattern="^[^\s@]+@[^\s@]+$" />"#,
            "RAZOR_ATTRIBUTE_TRANSITION",
        ),
        (
            r#"<input pattern="^[a-z]+@example\.com$" />"#,
            "RAZOR_ATTRIBUTE_TRANSITION",
        ),
        (r#"<input value="@(" />"#, "RAZOR_EXPRESSION_UNCLOSED"),
        (r#"<input value="@()" />"#, "RAZOR_ATTRIBUTE_EXPRESSION"),
        (
            r#"<input value="@Model.Items[0" />"#,
            "RAZOR_EXPRESSION_UNCLOSED",
        ),
    ] {
        assert_eq!(
            validate("x.cshtml", source, &ctx).unwrap_err().0,
            code,
            "{source}"
        );
    }
}

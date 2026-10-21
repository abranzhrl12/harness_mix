use std::time::Duration;

use harness_domain::execution::OperationContext;

use super::syntax::{collect, collect_cshtml_blocks, razor_markup_end, validate_syntax};

#[test]
fn csharp_extraction_ignores_braces_in_comments_and_strings_and_keeps_ranges() {
    let source = r#"/// documented
public class Sample {
    public string Name { get; set; }
    public void Run() {
        var text = "} class Hidden {}";
        /* } class AlsoHidden {} */
    }
}
"#;

    let symbols = collect(source, "ignored", &OperationContext::default()).unwrap();

    let names = symbols
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["Sample", "Name", "Run"]);
    let sample = &symbols[0];
    assert_eq!(sample.doc, "documented");
    assert!(source[sample.bytes.clone()].starts_with("class Sample"));
    assert!(source[sample.bytes.clone()].ends_with("\n}"));
    let run = symbols.iter().find(|symbol| symbol.name == "Run").unwrap();
    assert!(source[run.bytes.clone()].contains("class Hidden"));
    assert_eq!(run.start, 4);
    assert_eq!(run.end, 7);
}

#[test]
fn razor_markup_boundaries_respect_quoted_tags_comments_and_script_bodies() {
    let markup =
        br#"<div title=">">x</div><!-- @code { } --><script>const x = "@code { }";</script>@code"#;
    let start = 0;
    let tag_end = razor_markup_end(markup, start).unwrap();
    assert_eq!(&markup[start..tag_end], b"<div title=\">\">");

    let comment_start = markup
        .windows(4)
        .position(|window| window == b"<!--")
        .unwrap();
    let comment_end = razor_markup_end(markup, comment_start).unwrap();
    assert_eq!(&markup[comment_end - 3..comment_end], b"-->");

    let script_start = markup
        .windows(7)
        .position(|window| window == b"<script")
        .unwrap();
    let script_end = razor_markup_end(markup, script_start).unwrap();
    assert_eq!(&markup[script_end - 9..script_end], b"</script>");
}

#[test]
fn csharp_ranges_use_utf8_byte_offsets_and_crlf_line_numbers() {
    let source = "// λ\r\npublic class A {\r\n    public void B() {}\r\n}\r\n";
    let symbols = collect(source, "", &OperationContext::default()).unwrap();
    let class = symbols.iter().find(|symbol| symbol.name == "A").unwrap();
    let method = symbols.iter().find(|symbol| symbol.name == "B").unwrap();

    assert_eq!(class.start, 2);
    assert_eq!(
        &source[class.bytes.clone()],
        "class A {\r\n    public void B() {}\r\n}"
    );
    assert_eq!(method.start, 3);
    assert_eq!(
        &source.as_bytes()[method.bytes.clone()],
        b"public void B() {}"
    );
}

#[test]
fn razor_blocks_exclude_braces_and_ignore_markup_directive_lookalikes() {
    let source = concat!(
        "<!-- @code { var comment = true; } -->\n",
        "<script>@code { var script = true; }</script>\n",
        "@code {\n    public void Go() { var s = \"}\"; }\n}\n"
    );
    let mut blocks = Vec::new();

    let symbols = collect_cshtml_blocks(
        source,
        "digest",
        &OperationContext::default(),
        &mut blocks,
        true,
    )
    .unwrap();

    assert_eq!(blocks.len(), 1);
    let (range, tag) = &blocks[0];
    assert_eq!(*tag, "code");
    assert_eq!(
        &source[range.clone()],
        "\n    public void Go() { var s = \"}\"; }\n"
    );
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "Go");
    assert_eq!(
        &source[symbols[0].bytes.clone()],
        "public void Go() { var s = \"}\"; }"
    );
}

#[test]
fn parser_cancellation_and_size_limits_remain_enforced() {
    let cancelled = OperationContext::with_timeout(Duration::ZERO);
    assert!(collect("class A {}", "", &cancelled).is_err());

    let oversized = "x".repeat(4 * 1024 * 1024 + 1);
    let error = collect(&oversized, "", &OperationContext::default())
        .err()
        .unwrap();
    assert_eq!(error.0, "PARSER_LIMIT");
    let error = validate_syntax("large.cs", &oversized, &OperationContext::default())
        .err()
        .unwrap();
    assert_eq!(error.0, "FILE_TOO_LARGE");
}

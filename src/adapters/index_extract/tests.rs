use super::csharp::blank_csharp;
use super::decode::decode;
use super::orchestration::extract;
use codegraph_native::buffers::*;
use harness_domain::execution::OperationContext;

#[test]
fn preprocess_preserves_utf16_columns_crlf_and_both_branches() {
    let input = "  #if FLAG // 😀\r\nclass A {}\r\n#else\r\nclass B {}\r\n#endif\r\n#region hi\r\n";
    let output = blank_csharp(input).unwrap();
    assert_eq!(input.encode_utf16().count(), output.encode_utf16().count());
    assert!(output.contains("class A {}\r\n"));
    assert!(output.contains("class B {}\r\n"));
    assert!(!output.contains("#if"));
    assert!(output.contains("#region hi"));
    assert_eq!(blank_csharp("#iffy\n").unwrap(), "#iffy\n");
}

#[test]
fn csharp_extraction_accepts_eof_terminated_endregion() {
    let source = "namespace Example;\n#region \"Example\"\npublic class ExampleDto {}\n#endregion";
    assert!(codegraph_native::extract("Example.cs", source, "csharp").is_err());

    let output = blank_csharp(source).unwrap();

    assert!(output.ends_with("#endregion\n"));
    assert_eq!(source.lines().count(), output.lines().count());

    let graph = extract("Example.cs", source, &OperationContext::default()).unwrap();
    let nodes = graph["nodes"].as_array().unwrap();
    assert!(nodes.iter().any(|node| {
        node["name"].as_str() == Some("ExampleDto") && node["kind"].as_str() == Some("class")
    }));
    assert_eq!(nodes[0]["endLine"].as_u64(), Some(5));
}

#[test]
fn libro_diario_response_fixture_extracts_all_classes() {
    let fixture = include_str!("../../../test/LibroDiarioResponse.cs");
    // Exercise EOF normalization independently of how editors save the fixture.
    let unterminated = fixture.trim_end_matches(['\r', '\n']);
    let terminated = format!("{unterminated}\n");
    for source in [unterminated, terminated.as_str()] {
        let graph = extract(
            "test/LibroDiarioResponse.cs",
            source,
            &OperationContext::default(),
        )
        .unwrap();
        let nodes = graph["nodes"].as_array().unwrap();
        for expected in [
            "ReporteLibroDiarioResponse",
            "DatosReporteLibroDiarioResponse",
            "ReporteLibroDiarioPaginadoResponse",
            "MetaDataLibroDiario52",
            "ReporteLibroDiario52",
            "TotalesCuenta",
            "ReporteLibroDiario52SQL",
            "SeccionColumna",
            "FilaPivotada",
            "ReporteLibroDiario52Paginado",
            "LibroDiarioExportFila",
            "LibroDiarioExportMetaData",
            "LibroDiarioExportGenerales",
            "LibroDiarioExportResponse",
            "LibroDiario52ExportFila",
            "LibroDiario52ExportTotal",
            "LibroDiario52ExportGenerales",
            "LibroDiario52ExportResponse",
        ] {
            assert!(
                nodes.iter().any(|node| {
                    node["name"].as_str() == Some(expected)
                        && node["kind"].as_str() == Some("class")
                }),
                "missing class {expected}"
            );
        }
    }
}

#[test]
fn csharp_eof_normalization_is_limited_to_final_directives() {
    for source in ["class Plain {}", "class Plain {}\n"] {
        let prepared = blank_csharp(source).unwrap();
        assert_eq!(prepared, source);
        assert!(codegraph_native::extract("Plain.cs", &prepared, "csharp").is_ok());
    }

    for source in ["#region Plain\n#endregion", "class Plain {}\n#endregion"] {
        let prepared = blank_csharp(source).unwrap();
        assert!(prepared.ends_with('\n'), "{source}");
        assert_eq!(source.lines().count(), prepared.lines().count());
    }
    for source in [
        "#if DEBUG\nclass Plain {}\n#endif",
        "#if DEBUG\nclass Plain {}\n#else\nclass Other {}\n#endif",
    ] {
        let prepared = blank_csharp(source).unwrap();
        assert!(!prepared.contains("#if"));
        assert!(!prepared.contains("#else"));
        assert!(!prepared.contains("#endif"));
        assert_eq!(
            source.encode_utf16().count(),
            prepared.encode_utf16().count()
        );
        assert!(codegraph_native::extract("Conditional.cs", &prepared, "csharp").is_ok());
    }
}

#[test]
fn razor_csharp_projection_with_eof_endregion_still_extracts_members() {
    let source = "@code {\n    public void Run() {}\n    #region Members\n    public int Count { get; set; }\n    #endregion\n}";
    let result = extract("Views/Index.cshtml", source, &OperationContext::default()).unwrap();
    let nodes = result["nodes"].as_array().unwrap();
    assert!(
        nodes.iter().any(|node| {
            node["name"] == "Run" && node["kind"] == "method" && node["startLine"] == 2
        }),
        "{result}"
    );
    assert!(
        nodes.iter().any(|node| {
            node["name"] == "Count" && node["kind"] == "property" && node["startLine"] == 4
        }),
        "{result}"
    );
}

#[test]
fn malformed_legacy_javascript_recovers_literal_requests_with_explicit_confidence() {
    let source = "function broken( {\nfetch('/valid');\n";
    let graph = extract("legacy.js", source, &OperationContext::default()).unwrap();
    let references = graph["unresolvedReferences"].as_array().unwrap();
    let reference = references.iter().find(|r| r["referenceKind"] == "http_route").unwrap();
    let evidence: serde_json::Value =
        serde_json::from_str(reference["referenceName"].as_str().unwrap()).unwrap();
    assert_eq!(evidence["url"], "/valid");
    assert_eq!(evidence["method"], "GET");
    assert_eq!(evidence["extraction"], "recovered");
    assert!(evidence["confidence"].as_f64().unwrap() < 1.0);
    assert_eq!(reference["line"], 2);
}

#[test]
fn malformed_buffers_and_cancellation_fail_closed() {
    let raw = EmitOut {
        meta: vec![],
        nodes: vec![],
        edges: vec![],
        refs: vec![],
        arena: vec![],
    };
    assert!(decode(&raw, "x.rs", "rust", &OperationContext::default()).is_err());
    let context = OperationContext::default();
    context.cancel();
    assert_eq!(
        extract("x.rs", "fn x() {}", &context).unwrap_err().0,
        "CANCELLED"
    );
}

#[test]
fn scss_extraction_projects_rules_variables_and_nesting() {
    let source = "$brand: #1976d2;\n\n.card {\n  color: `$brand`;\n\n  &__title {\n    font-weight: 600;\n  }\n}\n";
    let graph = extract("card.scss", source, &OperationContext::default()).unwrap();
    let nodes = graph["nodes"].as_array().unwrap();
    assert!(
        nodes
            .iter()
            .any(|node| node["name"].as_str() == Some("$brand")
                && node["kind"].as_str() == Some("scss-variable")
                && node["language"].as_str() == Some("scss"))
    );
    assert!(
        nodes
            .iter()
            .any(|node| node["name"].as_str() == Some("&__title")
                && node["kind"].as_str() == Some("scss-rule")
                && node["startLine"].as_u64() == Some(6))
    );
    let edges = graph["edges"].as_array().unwrap();
    assert!(edges.iter().any(|edge| {
        edge["kind"].as_str() == Some("contains")
            && edge["source"].as_str() != edge["target"].as_str()
    }));
}

#[test]
fn python_extraction_projects_classes_methods_functions_imports_and_calls() {
    let source = "import sys\nfrom utils import run_helper\n\nclass Base:\n    def ping(self):\n        pass\n\nclass Service(Base):\n    def handle(self):\n        self.ping()\n        run_helper()\n\ndef main():\n    svc = Service()\n    svc.handle()\n";
    let graph = extract("app.py", source, &OperationContext::default()).unwrap();
    let nodes = graph["nodes"].as_array().unwrap();
    assert!(nodes.iter().any(|n| n["name"] == "Service" && n["kind"] == "class" && n["language"] == "python"));
    assert!(nodes.iter().any(|n| n["name"] == "handle" && n["kind"] == "method" && n["qualifiedName"] == "Service::handle"));
    assert!(nodes.iter().any(|n| n["name"] == "main" && n["kind"] == "function" && n["language"] == "python"));
    assert!(nodes.iter().any(|n| n["name"] == "sys" && n["kind"] == "import"));
    assert!(nodes.iter().any(|n| n["name"] == "utils" && n["kind"] == "import"));

    let refs = graph["unresolvedReferences"].as_array().unwrap();
    assert!(refs.iter().any(|r| r["referenceKind"] == "extends" && r["referenceName"] == "Base"));
    assert!(refs.iter().any(|r| r["referenceKind"] == "imports" && r["referenceName"] == "run_helper"));
    assert!(refs.iter().any(|r| r["referenceKind"] == "calls" && r["referenceName"] == "ping"));
    assert!(refs.iter().any(|r| r["referenceKind"] == "calls" && r["referenceName"] == "run_helper"));
    assert!(refs.iter().any(|r| r["referenceKind"] == "calls" && r["referenceName"] == "svc.handle"));
}#[test]
fn malformed_javascript_recovers_valid_oxc_statements_and_functions() {
    let source = "$(document).ready(function () {\n    var broken-name;\n    function init() {\n        fetch('/inventory');\n    }\n    broken-name = 1;\n});\n";
    let graph = extract("legacy.js", source, &OperationContext::default()).unwrap();
    assert!(graph["nodes"].as_array().unwrap().iter().any(|node| node["name"] == "init"));
    assert!(graph["unresolvedReferences"].as_array().unwrap().iter().any(|reference| {
        reference["referenceKind"] == "http_route"
            && reference["line"] == 4
            && reference["referenceName"].as_str().is_some_and(|name| name.contains("/inventory"))
    }));
    assert_eq!(graph["coverage"]["status"], "partial");
}

#[test]
fn numbers_current_source_without_changing_line_content() {
    let rendered = super::source("a.rs", 7, "fn a() {\r\n    b();\r\n}", false);
    assert!(rendered.contains("```rust\n7 | fn a() {\n8 |     b();\n9 | }"));
    assert!(!rendered.contains("truncated"));
}

#[test]
fn source_cannot_close_its_fence_and_truncation_is_explicit() {
    let rendered = super::source("a.rs", 1, "```\nhello", true);
    assert!(rendered.contains("\n````rust\n1 | ```"));
    assert!(rendered.contains("Source fragment truncated"));
}

#[test]
fn unknown_extensions_keep_a_bare_fence() {
    let rendered = super::source("notes.txt", 1, "yes", false);
    assert!(rendered.contains("```\n1 | yes"));
}

#[test]
fn summary_keeps_calls_ahead_of_type_references_and_reports_omissions() {
    use super::super::model::Node;
    use harness_domain::contracts::ExploreDirection;
    use serde_json::{Value, json};
    use std::collections::BTreeMap;

    let focus = Node {
        id: "focus".into(),
        name: "explore".into(),
        kind: "function".into(),
        file_path: "src/explore.rs".into(),
        start_line: 10,
    };
    let mut nodes = BTreeMap::from([("focus".to_owned(), focus.clone())]);
    let mut edges = Vec::<Value>::new();
    for index in 0..5 {
        let id = format!("type-{index}");
        let node = Node {
            id: id.clone(),
            name: format!("Type{index}"),
            kind: "struct".into(),
            file_path: format!("src/types{index}.rs"),
            start_line: (index + 1) as i64,
        };
        nodes.insert(id.clone(), node);
        edges.push(json!({
            "id": index + 1,
            "source": "focus",
            "target": id,
            "kind": "references"
        }));
    }
    let caller = Node {
        id: "caller".into(),
        name: "run".into(),
        kind: "function".into(),
        file_path: "src/caller.rs".into(),
        start_line: 3,
    };
    nodes.insert(caller.id.clone(), caller.clone());
    // Put the important edge last to reproduce storage order hiding it.
    edges.push(json!({
        "id": 6,
        "source": focus.id,
        "target": caller.id,
        "kind": "calls",
        "resolutionEvidence": {
            "provenance": "heuristic",
            "resolvedBy": "ra-module-subset-v1",
            "storedConfidence": 0.9
        }
    }));

    let summary = super::summary_graph(
        std::slice::from_ref(&focus),
        &nodes,
        &edges,
        false,
        json!({"targetSelection":false,"depthBoundary":false,"perNodeEdges":false,"totalEdges":false}),
        ExploreDirection::Both,
        1,
    )
    .unwrap();
    let relations = summary["relations"].as_array().unwrap();
    assert_eq!(relations.len(), 4);
    let call = relations.iter().find(|edge| edge["kind"] == "calls").unwrap();
    assert_eq!(call["resolutionEvidence"]["provenance"], "heuristic");
    assert_eq!(call["resolutionEvidence"]["resolvedBy"], "ra-module-subset-v1");
    assert_eq!(call["resolutionEvidence"]["storedConfidence"], 0.9);
    assert_eq!(summary["resolutionEvidence"]["recordedEdges"], 1);
    assert_eq!(summary["resolutionEvidence"]["unrecordedEdges"], 5);
    assert_eq!(summary["resolutionEvidence"]["validated"], false);
    assert_eq!(summary["relationsOmitted"], 2);
    assert_eq!(summary["summaryTruncated"], true);

    let mut reversed_edges = edges.clone();
    reversed_edges.reverse();
    let reordered = super::summary_graph(
        std::slice::from_ref(&focus),
        &nodes,
        &reversed_edges,
        false,
        json!({"targetSelection":false,"depthBoundary":false,"perNodeEdges":false,"totalEdges":false}),
        ExploreDirection::Both,
        1,
    )
    .unwrap();
    assert_eq!(summary, reordered);
}

#[test]
fn traces_t_observacion_empleado_end_to_end_across_form_json_csharp_sp_and_sql() {
    use super::super::data_trace;
    use crate::adapters::project::Project;
    use harness_domain::{
        contracts::{DataTraceLayerKind, ExploreArgs},
        execution::OperationContext,
    };
    use serde_json::json;
    use std::collections::BTreeSet;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("Views/Empleado")).unwrap();
    std::fs::create_dir_all(root.join("Scripts/Empleado")).unwrap();
    std::fs::create_dir_all(root.join("Models")).unwrap();
    std::fs::create_dir_all(root.join("Controllers")).unwrap();
    std::fs::create_dir_all(root.join("Sql")).unwrap();

    std::fs::write(
        root.join("Views/Empleado/Editar.cshtml"),
        r#"@model EmpleadoViewModel
<div class="form-group">
    <input type="text" id="tObservacionEmpleado" name="tObservacionEmpleado" asp-for="ObservacionEmpleado" value="@Model.tObservacionEmpleado" />
</div>
"#,
    )
    .unwrap();

    std::fs::write(
        root.join("Scripts/Empleado/editar.js"),
        r#"function guardarEmpleado() {
    const obs = $('#tObservacionEmpleado').val();
    const payload = { tObservacionEmpleado: obs };
    $.ajax({ url: '/Empleado/Guardar', data: JSON.stringify(payload) });
}
"#,
    )
    .unwrap();

    std::fs::write(
        root.join("Models/EmpleadoViewModel.cs"),
        r#"namespace WebApp.Models {
    public class EmpleadoViewModel {
        public string tObservacionEmpleado { get; set; }
    }
}
"#,
    )
    .unwrap();

    std::fs::write(
        root.join("Controllers/EmpleadoController.cs"),
        r#"using System.Data.SqlClient;
namespace WebApp.Controllers {
    public class EmpleadoController {
        public void Guardar(EmpleadoViewModel model) {
            var cmd = new SqlCommand("usp_GuardarEmpleado");
            cmd.Parameters.AddWithValue("@tObservacionEmpleado", model.tObservacionEmpleado);
        }
    }
}
"#,
    )
    .unwrap();

    std::fs::write(
        root.join("Sql/usp_GuardarEmpleado.sql"),
        r#"CREATE PROCEDURE dbo.usp_GuardarEmpleado
    @tObservacionEmpleado VARCHAR(250)
AS
BEGIN
    UPDATE dbo.Empleado
    SET ObservacionEmpleado = @tObservacionEmpleado
    WHERE IdEmpleado = 1;
END
GO
"#,
    )
    .unwrap();

    let project = Project::new(root).unwrap();
    let args: ExploreArgs = serde_json::from_value(json!({
        "query": "tObservacionEmpleado"
    }))
    .unwrap();
    let context = OperationContext::default();

    let trace = data_trace::trace(
        &project,
        &args,
        "tObservacionEmpleado",
        &[],
        &BTreeSet::new(),
        &context,
    )
    .unwrap()
    .expect("trace should be produced");

    assert_eq!(trace.identifier, "tObservacionEmpleado");
    assert_eq!(trace.normalized_identifier, "ObservacionEmpleado");
    assert_eq!(trace.parameter_name, "@tObservacionEmpleado");
    assert_eq!(
        trace.chain,
        "Form (.cshtml) → JSON/JS (.js) → C# Model/Controller (.cs) → SP Parameter (@...) → SQL Column (.sql)"
    );
    assert!(trace.complete_chain);
    assert_eq!(trace.matched_layer_count, 5);
    assert!(trace.missing_layers.is_empty());
    assert_eq!(
        trace.matched_layers,
        vec![
            DataTraceLayerKind::Form,
            DataTraceLayerKind::Json,
            DataTraceLayerKind::CsharpModel,
            DataTraceLayerKind::SpParameter,
            DataTraceLayerKind::SqlColumn,
        ]
    );

    let form_layer = &trace.layers[0];
    assert_eq!(form_layer.layer, DataTraceLayerKind::Form);
    assert!(form_layer.matched);
    assert_eq!(form_layer.matches[0].file_path, "Views/Empleado/Editar.cshtml");
    assert_eq!(form_layer.matches[0].line, 3);

    let json_layer = &trace.layers[1];
    assert_eq!(json_layer.layer, DataTraceLayerKind::Json);
    assert!(json_layer.matched);
    assert!(
        json_layer
            .matches
            .iter()
            .any(|m| m.file_path == "Scripts/Empleado/editar.js" && m.line == 2)
    );

    let cs_layer = &trace.layers[2];
    assert_eq!(cs_layer.layer, DataTraceLayerKind::CsharpModel);
    assert!(cs_layer.matched);
    assert!(
        cs_layer
            .matches
            .iter()
            .any(|m| m.file_path == "Models/EmpleadoViewModel.cs" && m.kind == "csharp_property")
    );

    let sp_layer = &trace.layers[3];
    assert_eq!(sp_layer.layer, DataTraceLayerKind::SpParameter);
    assert!(sp_layer.matched);
    assert!(
        sp_layer
            .matches
            .iter()
            .any(|m| m.file_path == "Controllers/EmpleadoController.cs" && m.kind == "add_with_value")
    );
    assert!(
        sp_layer
            .matches
            .iter()
            .any(|m| m.file_path == "Sql/usp_GuardarEmpleado.sql" && m.kind == "sp_parameter_declaration")
    );

    let sql_layer = &trace.layers[4];
    assert_eq!(sql_layer.layer, DataTraceLayerKind::SqlColumn);
    assert!(sql_layer.matched);
    assert!(
        sql_layer
            .matches
            .iter()
            .any(|m| m.file_path == "Sql/usp_GuardarEmpleado.sql" && m.line == 6)
    );

    let rendered = super::render_data_trace(&trace, false);
    assert!(rendered.contains("**Cross-Layer Data Lineage (`tObservacionEmpleado`)**"));
    assert!(rendered.contains(
        "Form (.cshtml) → JSON/JS (.js) → C# Model/Controller (.cs) → SP Parameter (@...) → SQL Column (.sql)"
    ));
    assert!(rendered.contains("Views/Empleado/Editar.cshtml:3"));
    assert!(rendered.contains("Scripts/Empleado/editar.js:2"));
    assert!(rendered.contains("Models/EmpleadoViewModel.cs:3"));
    assert!(rendered.contains("Controllers/EmpleadoController.cs:6"));
    assert!(rendered.contains("Sql/usp_GuardarEmpleado.sql:6"));
}

#[test]
fn data_trace_reports_missing_layers_explicitly_when_chain_is_incomplete() {
    use super::super::data_trace;
    use crate::adapters::project::Project;
    use harness_domain::{
        contracts::{DataTraceLayerKind, ExploreArgs},
        execution::OperationContext,
    };
    use serde_json::json;
    use std::collections::BTreeSet;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("Models")).unwrap();
    std::fs::write(
        root.join("Models/InventarioDto.cs"),
        "public class InventarioDto { public int iStockActual { get; set; } }\n",
    )
    .unwrap();

    let project = Project::new(root).unwrap();
    let args: ExploreArgs = serde_json::from_value(json!({
        "query": "iStockActual"
    }))
    .unwrap();
    let trace = data_trace::trace(
        &project,
        &args,
        "iStockActual",
        &[],
        &BTreeSet::new(),
        &OperationContext::default(),
    )
    .unwrap()
    .unwrap();

    assert!(!trace.complete_chain);
    assert_eq!(trace.matched_layer_count, 1);
    assert_eq!(trace.matched_layers, vec![DataTraceLayerKind::CsharpModel]);
    assert_eq!(
        trace.missing_layers,
        vec![
            DataTraceLayerKind::Form,
            DataTraceLayerKind::Json,
            DataTraceLayerKind::SpParameter,
            DataTraceLayerKind::SqlColumn,
        ]
    );
    let rendered = super::render_data_trace(&trace, false);
    assert!(rendered.contains("missing / unmatched"));
    assert!(rendered.contains("[✗ missing]"));
}

//! Projects provider facts into native registration nodes and relationship refs.
use super::{CONTRACT, Facts, LIMIT, analyze};
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};
pub(super) fn enrich(
    path: &str,
    source: &str,
    graph: &mut Value,
    context: &OperationContext,
) -> Result<(), Failure> {
    if !source.contains("@angular/core") || !source.contains("providers") {
        return Ok(());
    }
    let facts = analyze(path, source, context)?;
    project(path, source, graph, &facts, context)
}

fn project(
    path: &str,
    source: &str,
    graph: &mut Value,
    facts: &Facts,
    context: &OperationContext,
) -> Result<(), Failure> {
    let nodes = graph["nodes"].as_array().ok_or(CONTRACT)?;
    let language = nodes
        .iter()
        .find_map(|node| node["language"].as_str())
        .unwrap_or("typescript")
        .to_owned();
    let original_nodes = nodes.len();
    let original_refs = graph["unresolvedReferences"]
        .as_array()
        .ok_or(CONTRACT)?
        .len();
    let original_edges = graph["edges"].as_array().ok_or(CONTRACT)?.len();
    let mut added_nodes = Vec::new();
    let mut added_refs = Vec::new();
    let mut added_edges = Vec::new();
    let mut notices = facts.notices.clone();
    let positions = super::super::super::framework::Positions::new(source);
    for registration in &facts.registrations {
        context.check()?;
        let (class_line, class_column) = positions.position(registration.class_start)?;
        let class_ids = nodes
            .iter()
            .filter(|node| {
                node["kind"] == "class"
                    && node["name"] == registration.class_name
                    && node["startLine"] == class_line
                    && node["startColumn"] == class_column
            })
            .filter_map(|node| node["id"].as_str())
            .collect::<Vec<_>>();
        if class_ids.len() != 1 {
            notices.insert(format!(
                "Provider scope {} has no unique native class anchor; its registrations were not projected",
                registration.class_name
            ));
            continue;
        }
        let (start_line, start_column) = positions.position(registration.provider_start)?;
        let (end_line, end_column) = positions.position(registration.provider_end)?;
        let id = super::super::super::framework::id(
            "angular",
            path,
            "provider-registration",
            registration.provider_start,
        );
        added_nodes.push(json!({
            "id":id,
            "kind":"module",
            "name":format!("{} provider", registration.token.imported),
            "qualifiedName":format!("{path}::{}#provider@{}", registration.class_name, registration.provider_start),
            "filePath":path,
            "language":language,
            "startLine":start_line,
            "endLine":end_line,
            "startColumn":start_column,
            "endColumn":end_column,
            "signature":format!("Angular provider registration scope={} token={} {}={}", registration.class_name, registration.token.imported, registration.form, registration.implementation.imported),
        }));
        added_edges.push(json!({
            "source":class_ids[0],
            "target":id,
            "kind":"contains",
            "provenance":"heuristic"
        }));
        for (binding, relationship, reference_kind) in [
            (&registration.token, "token", "angular_provider_token"),
            (
                &registration.implementation,
                "implementation",
                "angular_provider_implementation",
            ),
        ] {
            added_refs.push(json!({
                "fromNodeId":id,
                "referenceName":binding.name,
                "referenceKind":reference_kind,
                "line":binding.line,
                "column":binding.column,
                "filePath":path,
                "metadata":{"framework":"angular","relationship":relationship,"module":binding.module,"imported":binding.imported,"registrationForm":registration.form}
            }));
        }
    }
    if original_nodes.saturating_add(added_nodes.len()) > 20_000
        || original_edges.saturating_add(added_edges.len()) > 20_000
        || original_refs.saturating_add(added_refs.len()) > 20_000
    {
        return Err(LIMIT);
    }
    let registrations_emitted = added_nodes.len();
    graph["nodes"]
        .as_array_mut()
        .ok_or(CONTRACT)?
        .append(&mut added_nodes);
    graph["unresolvedReferences"]
        .as_array_mut()
        .ok_or(CONTRACT)?
        .append(&mut added_refs);
    graph["edges"]
        .as_array_mut()
        .ok_or(CONTRACT)?
        .append(&mut added_edges);
    if !graph["coverage"].is_object() {
        graph["coverage"] = json!({});
    }
    graph["coverage"]["providers"] = json!({
        "parser":"angular-static-provider-registration-subset-v1",
        "registrationsEmitted":registrations_emitted,
        "relationships":["token","implementation"],
        "forms":["useClass","useExisting"],
        "runtimeInjectorSelectionEvaluated":false,
        "limitations":["Only direct Component, Directive, and NgModule providers arrays are inspected","Only unique imported value identifiers are captured; publication links only unique indexed class exports","providedIn scopes, multi providers, factory/value providers, computed properties, spreads, and dynamic expressions are unsupported"],
        "notices":notices.into_iter().collect::<Vec<_>>()
    });
    Ok(())
}

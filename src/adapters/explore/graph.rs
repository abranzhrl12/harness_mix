use super::model::Node;
use crate::adapters::project::Project;
use harness_domain::{Failure, contracts::ExploreDirection, execution::OperationContext};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    time::{Duration, Instant},
};

pub(super) struct GraphWalk {
    pub(super) nodes: BTreeMap<String, Node>,
    pub(super) edges: Vec<Value>,
    pub(super) export_contributors: BTreeMap<String, String>,
    pub(super) depths: BTreeMap<String, BTreeMap<&'static str, usize>>,
    pub(super) expanded: usize,
    pub(super) depth_boundary: bool,
    pub(super) truncated: bool,
    pub(super) per_node_edges_limited: bool,
    pub(super) total_edges_limited: bool,
}

pub(super) fn traverse(
    db: &Connection,
    project: &Project,
    context: &OperationContext,
    diagnostic: &std::cell::RefCell<Value>,
    started: Instant,
    targets: &[Node],
    direction: ExploreDirection,
    max_depth: usize,
    initially_truncated: bool,
) -> Result<GraphWalk, Failure> {
    let failed = |error| super::database::failure(error, diagnostic, context, started);
    let mut nodes: BTreeMap<String, Node> =
        targets.iter().map(|n| (n.id.clone(), n.clone())).collect();
    let mut edges = Vec::new();
    let mut edge_ids = BTreeSet::new();
    let mut export_contributors = BTreeMap::new();
    // Minimal legacy reader schemas need not have edge metadata. Detect the
    // column explicitly; never turn a malformed full-schema query into a
    // silent "no provenance" fallback.
    let has_metadata: i64 = db
        .query_row(
            "SELECT count(*) FROM pragma_table_info('edges') WHERE name='metadata'",
            [],
            |r| r.get(0),
        )
        .map_err(failed)?;
    let has_provenance: i64 = db
        .query_row(
            "SELECT count(*) FROM pragma_table_info('edges') WHERE name='provenance'",
            [],
            |r| r.get(0),
        )
        .map_err(failed)?;
    let evidence_sql = match (has_metadata == 1, has_provenance == 1) {
        (true, true) => Some(
            "SELECT CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.exportContributors') END, CASE WHEN json_valid(metadata) THEN CASE WHEN json_type(metadata,'$.resolvedBy')='text' AND length(json_extract(metadata,'$.resolvedBy'))<=256 THEN json_extract(metadata,'$.resolvedBy') END END, CASE WHEN json_valid(metadata) THEN CASE WHEN json_type(metadata,'$.confidence') IN ('integer','real') THEN json_extract(metadata,'$.confidence') END END, CASE WHEN length(provenance)<=128 THEN provenance END FROM edges WHERE id=?1",
        ),
        (true, false) => Some(
            "SELECT CASE WHEN json_valid(metadata) THEN json_extract(metadata,'$.exportContributors') END, CASE WHEN json_valid(metadata) THEN CASE WHEN json_type(metadata,'$.resolvedBy')='text' AND length(json_extract(metadata,'$.resolvedBy'))<=256 THEN json_extract(metadata,'$.resolvedBy') END END, CASE WHEN json_valid(metadata) THEN CASE WHEN json_type(metadata,'$.confidence') IN ('integer','real') THEN json_extract(metadata,'$.confidence') END END, NULL FROM edges WHERE id=?1",
        ),
        (false, true) => Some(
            "SELECT NULL, NULL, NULL, CASE WHEN length(provenance)<=128 THEN provenance END FROM edges WHERE id=?1",
        ),
        (false, false) => None,
    };
    let mut evidence_query = evidence_sql
        .map(|query| db.prepare(query).map_err(failed))
        .transpose()?;
    let directions: &[bool] = match direction {
        ExploreDirection::Inbound => &[true],
        ExploreDirection::Outbound => &[false],
        ExploreDirection::Both => &[true, false],
    };
    let mut frontier = VecDeque::new();
    let mut depths: BTreeMap<String, BTreeMap<&'static str, usize>> = BTreeMap::new();
    for target in targets {
        for &incoming in directions {
            let label = if incoming { "inbound" } else { "outbound" };
            let previous = depths
                .entry(target.id.clone())
                .or_default()
                .insert(label, 0);
            if previous.is_none() {
                frontier.push_back((target.id.clone(), 0, incoming));
            }
        }
    }
    let mut expanded = 0;
    let mut depth_boundary = false;
    let mut truncated = initially_truncated;
    let mut per_node_edges_limited = false;
    let mut total_edges_limited = false;
    // Each direction has its own BFS frontier. Inbound traversal never turns
    // outbound at an intermediate node (which would introduce sibling callers).
    'traversal: while let Some((current, depth, incoming)) = frontier.pop_front() {
        if depth >= max_depth {
            depth_boundary = true;
            continue;
        }
        context.check()?;
        if started.elapsed() >= Duration::from_secs(5) {
            return Err(super::BUDGET);
        }
        expanded += 1;
        // SQL text and edge kinds are closed constants, never request text.
        let sql = if incoming {
            "SELECT e.id,e.source,e.target,e.kind,n.id,n.name,n.kind,n.file_path,n.start_line FROM edges e JOIN nodes n ON n.id=e.source WHERE e.target=?1 AND e.kind IN ('calls','references','imports','instantiates','navigates') ORDER BY e.id LIMIT 21"
        } else {
            "SELECT e.id,e.source,e.target,e.kind,n.id,n.name,n.kind,n.file_path,n.start_line FROM edges e JOIN nodes n ON n.id=e.target WHERE e.source=?1 AND e.kind IN ('calls','references','imports','instantiates','navigates') ORDER BY e.id LIMIT 21"
        };
        let mut statement = db.prepare(sql).map_err(failed)?;
        let mut rows = statement.query([&current]).map_err(failed)?;
        let mut count = 0;
        while let Some(row) = rows.next().map_err(failed)? {
            context.check()?;
            count += 1;
            if started.elapsed() >= Duration::from_secs(5) {
                return Err(super::BUDGET);
            }
            if count > 20 {
                truncated = true;
                per_node_edges_limited = true;
                break;
            }
            let id: i64 = row.get(0).map_err(failed)?;
            let source: String = row.get(1).map_err(failed)?;
            let destination: String = row.get(2).map_err(failed)?;
            let kind: String = row.get(3).map_err(failed)?;
            let mut related = super::node(row, 4).map_err(failed)?;
            super::validate_node(project, &mut related)?;
            if source.len() > 1024 || destination.len() > 1024 {
                return Err(super::CONTRACT);
            }
            if !edge_ids.contains(&id) && edges.len() >= 160 {
                truncated = true;
                total_edges_limited = true;
                break 'traversal;
            }
            let label = if incoming { "inbound" } else { "outbound" };
            let known = depths.entry(related.id.clone()).or_default();
            if let std::collections::btree_map::Entry::Vacant(entry) = known.entry(label) {
                entry.insert(depth + 1);
                depth_boundary |= depth + 1 == max_depth;
                frontier.push_back((related.id.clone(), depth + 1, incoming));
            }
            nodes.insert(related.id.clone(), related);
            if edge_ids.insert(id) {
                let edge_evidence = evidence_query
                    .as_mut()
                    .map(|query| {
                        query.query_row([id], |row| {
                            Ok((
                                row.get::<_, Option<String>>(0)?,
                                row.get::<_, Option<String>>(1)?,
                                row.get::<_, Option<f64>>(2)?,
                                row.get::<_, Option<String>>(3)?,
                            ))
                        })
                    })
                    .transpose()
                    .map_err(failed)?
                    .unwrap_or((None, None, None, None));
                super::super::index_freshness::export_contributors(
                    edge_evidence.0.as_deref(),
                    &mut export_contributors,
                )?;
                let mut resolution_evidence = serde_json::Map::new();
                if let Some(provenance) = edge_evidence.3 {
                    resolution_evidence.insert("provenance".into(), json!(provenance));
                }
                if let Some(resolved_by) = edge_evidence.1 {
                    resolution_evidence.insert("resolvedBy".into(), json!(resolved_by));
                }
                if let Some(confidence) = edge_evidence.2.filter(|value| value.is_finite()) {
                    resolution_evidence.insert("storedConfidence".into(), json!(confidence));
                }
                let mut edge = json!({"id":id,"source":source,"target":destination,"kind":kind});
                if !resolution_evidence.is_empty() {
                    edge["resolutionEvidence"] = serde_json::Value::Object(resolution_evidence);
                }
                edges.push(edge);
            }
        }
    }
    Ok(GraphWalk {
        nodes,
        edges,
        export_contributors,
        depths,
        expanded,
        depth_boundary,
        truncated,
        per_node_edges_limited,
        total_edges_limited,
    })
}

//! Shared assembly for the SQL schema map.
//!
//! Both engines answer `action=map` with the same flat row shape and this
//! module turns those rows into one compact structure: relations with their
//! primary key, foreign key edges and routine parameters. The map exists so
//! an agent learns the shape of a database in a single round trip instead of
//! one search plus one describe per table.

use serde_json::{Value, json};

/// Normalize a SQL parameter or column name for cross-layer lineage matching:
/// strips leading `@` and common Hungarian type prefixes (`t`, `i`, `n`, `b`, `f`, `d`)
/// when followed by an uppercase ASCII letter (e.g., `@tObservacionEmpleado` -> `ObservacionEmpleado`).
pub(crate) fn normalize_sql_identifier(raw: &str) -> (String, String, String) {
    let clean = raw.trim().trim_start_matches('@').to_owned();
    let bytes = clean.as_bytes();
    let normalized = if bytes.len() >= 2
        && matches!(bytes[0], b't' | b'i' | b'n' | b'b' | b'f' | b'd')
        && bytes[1].is_ascii_uppercase()
    {
        clean[1..].to_owned()
    } else {
        clean.clone()
    };
    let parameter = format!("@{clean}");
    (clean, normalized, parameter)
}

/// Checks whether two SQL/application identifiers match either directly (case-insensitively,
/// ignoring `@`) or via their Hungarian-stripped stem.
pub(crate) fn sql_identifiers_match(a: &str, b: &str) -> bool {
    let (clean_a, norm_a, _) = normalize_sql_identifier(a);
    let (clean_b, norm_b, _) = normalize_sql_identifier(b);
    if clean_a.is_empty() || clean_b.is_empty() {
        return false;
    }
    clean_a.eq_ignore_ascii_case(&clean_b)
        || (norm_a.len() >= 2
            && norm_b.len() >= 2
            && norm_a.eq_ignore_ascii_case(&norm_b))
}

/// One normalized catalog row. Both engines project their catalog into this
/// shape before assembly.
pub(super) struct MapRow {
    pub(super) kind: String,
    pub(super) schema: String,
    pub(super) name: String,
    pub(super) detail1: String,
    pub(super) detail2: String,
    pub(super) detail3: String,
    pub(super) detail4: String,
    pub(super) number: Option<i64>,
}

impl MapRow {
    pub(super) fn new(
        kind: &str,
        schema: &str,
        name: &str,
        detail1: &str,
        detail2: &str,
        detail3: &str,
        detail4: &str,
    ) -> Self {
        Self {
            kind: kind.to_owned(),
            schema: schema.to_owned(),
            name: name.to_owned(),
            detail1: detail1.to_owned(),
            detail2: detail2.to_owned(),
            detail3: detail3.to_owned(),
            detail4: detail4.to_owned(),
            number: None,
        }
    }

    pub(super) fn with_number(mut self, number: Option<i64>) -> Self {
        self.number = number;
        self
    }
}

/// A foreign key edge: the constraint name, the child table and its columns,
/// and the parent table and its columns.
struct ForeignEdge {
    name: String,
    child: String,
    child_columns: Vec<String>,
    parent: String,
    parent_columns: Vec<String>,
}

/// Builds the map payload from catalog rows.
///
/// `limit` bounds the number of reported relations. Anything the catalog
/// returned beyond that bound is reported as truncated instead of dropped
/// silently.
pub(super) fn assemble(engine: &str, rows: Vec<MapRow>, limit: usize) -> Value {
    let mut relations: Vec<Value> = Vec::new();
    let mut primary_keys: Vec<(String, String, String)> = Vec::new();
    let mut foreign_keys: Vec<Value> = Vec::new();
    let mut routines: Vec<Value> = Vec::new();
    let mut relation_names: Vec<(String, String)> = Vec::new();
    let mut foreign_edges: Vec<ForeignEdge> = Vec::new();
    let mut routine_rows: Vec<(String, String, String, String, String, String)> = Vec::new();
    let mut catalog_rows = 0usize;
    let mut dropped_relations = false;

    for row in rows {
        catalog_rows += 1;
        match row.kind.as_str() {
            "relation" => {
                if relation_names.len() >= limit {
                    dropped_relations = true;
                    continue;
                }
                relation_names.push((row.schema.clone(), row.name.clone()));
                relations.push(json!({
                    "schema": row.schema,
                    "name": row.name,
                    "kind": row.detail1,
                    "rowsApprox": row.number.unwrap_or(0),
                    "primaryKey": []
                }));
            }
            "primaryKey" => {
                primary_keys.push((row.schema, row.name, row.detail1));
            }
            "foreignKey" => {
                let edge = foreign_edges
                    .iter_mut()
                    .find(|edge| edge.name == row.name && edge.child == row.detail1);
                match edge {
                    Some(edge) => {
                        edge.child_columns.push(row.detail2.clone());
                        edge.parent_columns.push(row.detail4.clone());
                    }
                    None => foreign_edges.push(ForeignEdge {
                        name: row.name.clone(),
                        child: row.detail1.clone(),
                        child_columns: vec![row.detail2.clone()],
                        parent: row.detail3.clone(),
                        parent_columns: vec![row.detail4.clone()],
                    }),
                }
            }
            "routine" => {
                routine_rows.push((
                    row.schema,
                    row.name,
                    row.detail1,
                    row.detail2,
                    row.detail3,
                    row.detail4,
                ));
            }
            _ => {}
        }
    }

    for (schema, table, column) in primary_keys {
        if let Some(relation) = relations
            .iter_mut()
            .find(|relation| relation["schema"] == schema && relation["name"] == table)
        {
            relation["primaryKey"]
                .as_array_mut()
                .expect("primary key array")
                .push(Value::String(column));
        }
    }

    for edge in foreign_edges {
        foreign_keys.push(json!({
            "name": edge.name,
            "child": edge.child,
            "childColumns": edge.child_columns,
            "parent": edge.parent,
            "parentColumns": edge.parent_columns
        }));
    }

    let mut routine_index: Vec<(String, String, Value)> = Vec::new();
    for (schema, name, parameter, data_type, direction, routine_kind) in routine_rows {
        let position = match routine_index
            .iter()
            .position(|(s, n, _)| *s == schema && *n == name)
        {
            Some(position) => position,
            None => {
                routine_index.push((
                    schema.clone(),
                    name.clone(),
                    json!({
                        "schema": schema,
                        "name": name,
                        "kind": routine_kind,
                        "parameters": []
                    }),
                ));
                routine_index.len() - 1
            }
        };
        if parameter.is_empty() && data_type.is_empty() {
            continue;
        }
        let (_, normalized_name, _) = normalize_sql_identifier(&parameter);
        routine_index[position].2["parameters"]
            .as_array_mut()
            .expect("parameter array")
            .push(json!({
                "name": parameter,
                "normalizedName": normalized_name,
                "type": data_type,
                "direction": if direction == "out" || direction == "inout" { "out" } else { "in" }
            }));
    }
    routines.extend(routine_index.into_iter().map(|(_, _, value)| value));

    json!({
        "engine": engine,
        "relations": relations,
        "foreignKeys": foreign_keys,
        "routines": routines,
        "counts": {
            "relations": relations.len(),
            "foreignKeys": foreign_keys.len(),
            "routines": routines.len()
        },
        "catalogRows": catalog_rows,
        "truncated": dropped_relations,
        "absenceIsConclusive": false
    })
}

#[cfg(test)]
#[path = "sql_map_tests.rs"]
mod tests;

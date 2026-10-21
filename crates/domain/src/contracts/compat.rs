use serde_json::Value;

#[cfg(feature = "schema")]
fn add_schema_alias(schema: &mut schemars::Schema, canonical: &str, alias: &str) {
    let object = schema.as_object_mut().expect("object schema");
    if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
        if let Some(property) = properties.get(canonical).cloned() {
            properties.insert(alias.to_string(), property);
        }
    }
    let required = object.get_mut("required").and_then(Value::as_array_mut);
    let was_required = required.is_some_and(|required| {
        let present = required.iter().any(|value| value == canonical);
        required.retain(|value| value != canonical);
        present
    });
    let constraints = object
        .entry("allOf")
        .or_insert_with(|| serde_json::json!([]));
    let constraints = constraints.as_array_mut().expect("allOf array");
    constraints.push(serde_json::json!({"not":{"required":[canonical,alias]}}));
    if was_required {
        constraints
            .push(serde_json::json!({"anyOf":[{"required":[canonical]},{"required":[alias]}]}));
    }
}

#[cfg(feature = "schema")]
pub(super) fn read_alias_schema(schema: &mut schemars::Schema) {
    // target_path predates file; one selector is required and multiple selectors are invalid.
    add_schema_alias(schema, "path", "file");
    add_schema_alias(schema, "path", "target_path");
    let object = schema.as_object_mut().expect("object schema");
    let constraints = object.get_mut("allOf").unwrap().as_array_mut().unwrap();
    // Replace the initial two-way requirement with all three accepted names.
    constraints.retain(|value| value.get("anyOf").is_none());
    constraints.push(serde_json::json!({"oneOf":[{"required":["path"]},{"required":["file"]},{"required":["target_path"]}]}));
}

#[cfg(feature = "schema")]
pub(super) fn batch_job_alias_schema(schema: &mut schemars::Schema) {
    add_schema_alias(schema, "arguments", "args");
}

#[cfg(feature = "schema")]
pub(super) fn search_alias_schema(schema: &mut schemars::Schema) {
    add_schema_property_alias(schema, "scope", "path");
    add_schema_alias(schema, "globs", "includes");
    add_schema_alias(schema, "contextLines", "context_lines");
}

#[cfg(feature = "schema")]
fn add_schema_property_alias(schema: &mut schemars::Schema, canonical: &str, alias: &str) {
    let object = schema.as_object_mut().expect("object schema");
    if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
        if let Some(property) = properties.get(canonical).cloned() {
            properties.insert(alias.to_string(), property);
        }
    }
}

#[cfg(feature = "schema")]
pub(super) fn replace_chunk_alias_schema(schema: &mut schemars::Schema) {
    add_schema_alias(schema, "oldText", "old_text");
    add_schema_alias(schema, "newText", "new_text");
}

#[cfg(feature = "schema")]
pub(super) fn replace_alias_schema(schema: &mut schemars::Schema) {
    add_schema_alias(schema, "hash", "baseHash");
    for (canonical, alias) in [
        ("oldText", "old_text"),
        ("newText", "new_text"),
        ("allowUnvalidatedText", "allow_unvalidated_text"),
    ] {
        add_schema_alias(schema, canonical, alias);
    }
}

/// Harmless envelope keys some clients add; never part of these tool arguments.
const INOCUOUS_FIELDS: &[&str] = &["action", "object", "objectType"];

pub(super) fn strip_inocuous(value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        for key in INOCUOUS_FIELDS {
            object.remove(*key);
        }
    }
}

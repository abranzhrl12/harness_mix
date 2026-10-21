//! JSON Schema adjustments that mirror serde's SQL contract aliases/defaults.

#[cfg(feature = "schema")]
pub(crate) fn describe_kind_aliases(schema: &mut schemars::Schema) {
    schema.insert(
        "enum".into(),
        serde_json::json!([
            "relation",
            "routine",
            "table",
            "view",
            "procedure",
            "function"
        ]),
    );
}

#[cfg(feature = "schema")]
pub(crate) fn describe_field_alias(schema: &mut schemars::Schema) {
    if let Some(properties) = schema
        .get_mut("properties")
        .and_then(serde_json::Value::as_object_mut)
    {
        if let Some(kind) = properties.get("kind").cloned() {
            properties.insert("objectType".into(), kind);
        }
    }
    // As with serde alias decoding, callers must choose one spelling.
    schema.insert(
        "not".into(),
        serde_json::json!({"required":["kind", "objectType"]}),
    );
}

/// Keep the advertised query branch consistent with implicit action=query.
#[cfg(feature = "schema")]
pub(crate) fn optional_query_action(schema: &mut schemars::Schema) {
    for key in ["oneOf", "anyOf"] {
        if let Some(branches) = schema
            .get_mut(key)
            .and_then(serde_json::Value::as_array_mut)
        {
            for branch in branches {
                if branch["properties"].get("sql").is_some() {
                    if let Some(required) = branch["required"].as_array_mut() {
                        required.retain(|field| field != "action");
                    }
                }
            }
        }
    }
}

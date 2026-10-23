use serde_json::{Map, Value, json};

const MAX_SAMPLE_PATHS_PER_CATEGORY: usize = 5;
const MAX_CATEGORY_SAMPLE_BYTES: usize = 2 * 1024;
const MAX_TOTAL_SAMPLE_BYTES: usize = 6 * 1024;
const MAX_SUMMARY_JSON_BYTES: usize = 12 * 1024;

const CATEGORIES: [(&str, &str); 6] = [
    ("added", "added"),
    ("modified", "modified"),
    ("retryExtraction", "retryExtraction"),
    ("removed", "removed"),
    ("unchanged", "unchanged"),
    ("unchecked", "unchecked"),
];

struct CategorySummary {
    name: &'static str,
    count: usize,
    sample_paths: Vec<(String, usize)>,
    sample_bytes: usize,
}

pub(super) fn summarize(result: &Value) -> Result<Value, String> {
    let plan = object_at(result, "plan")?;
    let inventory = object_at(result, "inventory")?;
    let coverage = object_at(result, "coverage")?;

    let mut sample_budget = MAX_TOTAL_SAMPLE_BYTES;
    let mut categories = Vec::with_capacity(CATEGORIES.len());
    for (name, field) in CATEGORIES {
        let paths = plan
            .get(field)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("plan.{field} is not an array"))?;
        let mut sample_paths = Vec::new();
        let mut sample_bytes = 0usize;
        for path in paths.iter().take(MAX_SAMPLE_PATHS_PER_CATEGORY) {
            let path = path
                .as_str()
                .ok_or_else(|| format!("plan.{field} contains a non-string path"))?;
            let encoded = serde_json::to_vec(path)
                .map_err(|error| format!("cannot encode plan.{field} sample: {error}"))?;
            let cost = encoded.len().saturating_add(1); // Reserve a comma per sample.
            if cost > MAX_CATEGORY_SAMPLE_BYTES.saturating_sub(sample_bytes) || cost > sample_budget
            {
                continue;
            }
            sample_bytes += cost;
            sample_budget -= cost;
            sample_paths.push((path.to_owned(), cost));
        }
        categories.push(CategorySummary {
            name,
            count: paths.len(),
            sample_paths,
            sample_bytes,
        });
    }

    let mut output = project(result, plan, inventory, coverage, &categories)?;
    let mut encoded = serde_json::to_vec(&output)
        .map_err(|error| format!("cannot encode plan summary: {error}"))?;
    while encoded.len() > MAX_SUMMARY_JSON_BYTES {
        let Some(category) = categories
            .iter_mut()
            .rev()
            .find(|category| !category.sample_paths.is_empty())
        else {
            return Err("plan summary metadata exceeds the JSON byte limit".into());
        };
        let (_, bytes) = category.sample_paths.pop().expect("non-empty sample list");
        category.sample_bytes -= bytes;
        output = project(result, plan, inventory, coverage, &categories)?;
        encoded = serde_json::to_vec(&output)
            .map_err(|error| format!("cannot encode plan summary: {error}"))?;
    }
    Ok(output)
}

fn project(
    result: &Value,
    plan: &Map<String, Value>,
    inventory: &Map<String, Value>,
    coverage: &Map<String, Value>,
    categories: &[CategorySummary],
) -> Result<Value, String> {
    let result = result
        .as_object()
        .ok_or("plan result is not a JSON object")?;
    let mut category_values = Map::new();
    let mut total_sample_bytes = 0usize;
    for category in categories {
        total_sample_bytes += category.sample_bytes;
        category_values.insert(
            category.name.into(),
            json!({
                "count": category.count,
                "samplePaths": category.sample_paths.iter().map(|(path, _)| path).collect::<Vec<_>>(),
                "omittedCount": category.count - category.sample_paths.len(),
            }),
        );
    }

    let inventory_summary = json!({
        "discoveryComplete": boolean(inventory, "discoveryComplete")?,
        "visitedEntries": integer(inventory, "visitedEntries")?,
        "reservedSourceBytes": integer(inventory, "reservedSourceBytes")?,
        "skippedEntries": integer(inventory, "skippedEntries")?,
        "restrictedIndexedFiles": integer(inventory, "restrictedIndexedFiles")?,
        "budgetExhausted": boolean(inventory, "budgetExhausted")?,
        "scopeExcludedCount": array_len(inventory, "scopeExcluded")?,
        "fileDiagnosticsCount": array_len(inventory, "fileDiagnostics")?,
    });
    let coverage_summary = json!({
        "trackedLanguages": string(coverage, "trackedLanguages")?,
        "newFileLanguages": string_array(coverage, "newFileLanguages")?,
        "fullProviderScope": boolean(coverage, "fullProviderScope")?,
        "providerConfigApplied": boolean(coverage, "providerConfigApplied")?,
        "trackedFilesCheckedOutsideDiscoveryIgnores": boolean(coverage, "trackedFilesCheckedOutsideDiscoveryIgnores")?,
        "deletionsRequireExplicitAbsenceOrConfigExclusion": boolean(coverage, "deletionsRequireExplicitAbsenceOrConfigExclusion")?,
        "limitations": string_array(coverage, "limitations")?,
    });

    let summary = json!({
        "inventoryComplete": boolean(plan, "inventoryComplete")?,
        "planSynchronized": boolean(plan, "synchronized")?,
        "maxPathsPerCategory": MAX_SAMPLE_PATHS_PER_CATEGORY,
        "samplePathJsonBudgetBytesUsed": total_sample_bytes,
        "categories": category_values,
        "inventory": inventory_summary,
        "coverage": coverage_summary,
        "detailsIncluded": false,
        "omittedSections": [
            "scope details",
            "inventory observations",
            "inventory scopeExcluded paths",
            "inventory fileDiagnostics",
            "inventory restrictedIndexedReasons",
            "extractionPreview",
            "plan paths outside category samples",
        ],
    });
    let mut projected = Map::new();
    for key in ["mode", "format"] {
        projected.insert(key.into(), Value::String(string(result, key)?));
    }
    projected.insert("schemaVersion".into(), number(result, "schemaVersion")?);
    projected.insert(
        "writesPerformed".into(),
        Value::Bool(boolean(result, "writesPerformed")?),
    );
    projected.insert(
        "synchronized".into(),
        Value::Bool(boolean(result, "synchronized")?),
    );
    projected.insert("summary".into(), summary);
    Ok(Value::Object(projected))
}

fn object_at<'a>(value: &'a Value, key: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("plan result has no object field {key}"))
}

fn boolean(object: &Map<String, Value>, key: &str) -> Result<bool, String> {
    object
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("plan summary field {key} is not a boolean"))
}

fn integer(object: &Map<String, Value>, key: &str) -> Result<u64, String> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("plan summary field {key} is not an unsigned integer"))
}

fn string(object: &Map<String, Value>, key: &str) -> Result<String, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("plan summary field {key} is not a string"))
}

fn string_array(object: &Map<String, Value>, key: &str) -> Result<Vec<String>, String> {
    object
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("plan summary field {key} is not an array"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("plan summary field {key} contains a non-string"))
        })
        .collect()
}

fn array_len(object: &Map<String, Value>, key: &str) -> Result<usize, String> {
    object
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::len)
        .ok_or_else(|| format!("plan summary field {key} is not an array"))
}

fn number(object: &Map<String, Value>, key: &str) -> Result<Value, String> {
    object
        .get(key)
        .filter(|value| value.is_number())
        .cloned()
        .ok_or_else(|| format!("plan summary field {key} is not numeric"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Value {
        json!({
            "mode": "plan_only",
            "format": "codegraph-sqlite-readonly-v1",
            "schemaVersion": 11,
            "writesPerformed": false,
            "synchronized": false,
            "plan": {
                "added": [],
                "modified": [],
                "retryExtraction": [],
                "removed": [],
                "unchanged": [],
                "unchecked": [],
                "inventoryComplete": false,
                "synchronized": false
            },
            "inventory": {
                "discoveryComplete": true,
                "visitedEntries": 0,
                "reservedSourceBytes": 0,
                "skippedEntries": 0,
                "restrictedIndexedFiles": 0,
                "restrictedIndexedReasons": {"symlink": 2},
                "budgetExhausted": false,
                "scopeExcluded": [],
                "fileDiagnostics": [],
                "observations": {"secret/path.rs": "ignored"},
                "scope": {"absoluteRoot": "hidden"}
            },
            "coverage": {
                "trackedLanguages": "all_nonrestricted",
                "newFileLanguages": ["rust", "typescript"],
                "fullProviderScope": false,
                "providerConfigApplied": true,
                "trackedFilesCheckedOutsideDiscoveryIgnores": true,
                "deletionsRequireExplicitAbsenceOrConfigExclusion": true,
                "limitations": ["bounded coverage"]
            },
            "scope": {"root": "must not copy"},
            "extractionPreview": {"heavy": true}
        })
    }

    fn sample<'a>(summary: &'a Value, category: &str) -> &'a Value {
        &summary["summary"]["categories"][category]
    }

    #[test]
    fn counts_are_exact_for_zero_one_five_six_and_many_paths() {
        let mut result = fixture();
        result["plan"]["added"] = json!([]);
        result["plan"]["modified"] = json!(["one.rs"]);
        result["plan"]["retryExtraction"] = json!(["a.rs", "b.rs", "c.rs", "d.rs", "e.rs"]);
        result["plan"]["removed"] = json!(["1", "2", "3", "4", "5", "6"]);
        result["plan"]["unchanged"] = json!(
            (0..27)
                .map(|index| format!("unchanged/{index}.rs"))
                .collect::<Vec<_>>()
        );

        let summary = summarize(&result).unwrap();
        assert_eq!(sample(&summary, "added")["count"], 0);
        assert_eq!(sample(&summary, "added")["omittedCount"], 0);
        assert_eq!(sample(&summary, "modified")["count"], 1);
        assert_eq!(sample(&summary, "retryExtraction")["count"], 5);
        assert_eq!(sample(&summary, "removed")["count"], 6);
        assert_eq!(
            sample(&summary, "removed")["samplePaths"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
        assert_eq!(sample(&summary, "removed")["omittedCount"], 1);
        assert_eq!(sample(&summary, "unchanged")["count"], 27);
        assert_eq!(sample(&summary, "unchanged")["omittedCount"], 22);
        assert_eq!(sample(&summary, "unchecked")["count"], 0);
    }

    #[test]
    fn preserves_input_order_unicode_and_round_trips_json() {
        let mut result = fixture();
        result["plan"]["added"] = json!(["z/雪.rs", "a/é.rs", "m/ç.rs"]);
        let summary = summarize(&result).unwrap();
        assert_eq!(
            sample(&summary, "added")["samplePaths"],
            json!(["z/雪.rs", "a/é.rs", "m/ç.rs"])
        );
        assert_eq!(summary["summary"]["detailsIncluded"], false);
        assert!(summary["scope"].is_null());
        assert!(summary["extractionPreview"].is_null());
        let json_text = serde_json::to_string(&summary).unwrap();
        let decoded: Value = serde_json::from_str(&json_text).unwrap();
        assert_eq!(decoded, summary);
    }

    #[test]
    fn rejects_missing_or_malformed_plan_categories_instead_of_zeroing_them() {
        let mut missing = fixture();
        missing["plan"].as_object_mut().unwrap().remove("added");
        assert!(summarize(&missing).is_err());

        let mut malformed = fixture();
        malformed["plan"]["modified"] = json!("not an array");
        assert!(summarize(&malformed).is_err());
    }

    #[test]
    fn omits_whole_long_paths_to_stay_within_serialized_byte_budgets() {
        let mut result = fixture();
        let mut long_unicode_paths = (0..5)
            .map(|index| format!("dir/{index}/{}", "雪".repeat(330)))
            .collect::<Vec<_>>();
        long_unicode_paths.push("short/late.rs".into());
        result["plan"]["added"] = json!(long_unicode_paths);
        let long_paths = json!(
            (0..60)
                .map(|i| format!("m/{i}/{}", "x".repeat(700)))
                .collect::<Vec<_>>()
        );
        result["plan"]["modified"] = long_paths.clone();
        result["plan"]["retryExtraction"] = long_paths.clone();
        result["plan"]["removed"] = long_paths.clone();
        result["plan"]["unchanged"] = long_paths.clone();
        result["plan"]["unchecked"] = long_paths;

        let summary = summarize(&result).unwrap();
        let encoded = serde_json::to_vec(&summary).unwrap();
        assert!(encoded.len() <= MAX_SUMMARY_JSON_BYTES);
        assert_eq!(sample(&summary, "added")["count"], 6);
        assert_eq!(
            sample(&summary, "added")["samplePaths"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(sample(&summary, "added")["omittedCount"], 4);
        assert!(
            sample(&summary, "added")["samplePaths"][0]
                .as_str()
                .unwrap()
                .contains('雪')
        );
        assert!(
            !sample(&summary, "added")["samplePaths"]
                .as_array()
                .unwrap()
                .contains(&json!("short/late.rs"))
        );
        for (_, field) in CATEGORIES {
            assert!(summary["summary"]["categories"][field]["omittedCount"].is_number());
            assert!(summary["summary"]["categories"][field]["count"].is_number());
            assert!(
                summary["summary"]["categories"][field]["samplePaths"]
                    .as_array()
                    .is_some()
            );
        }
        assert_eq!(summary["summary"]["inventory"]["scopeExcludedCount"], 0);
        assert_eq!(summary["summary"]["inventory"]["fileDiagnosticsCount"], 0);
        assert!(summary["summary"]["omittedSections"].as_array().is_some());
    }
}

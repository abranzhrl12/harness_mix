use super::super::index_sync::Observation;
use super::*;
use harness_application::indexing::inventory::MAX_BYTES;
use std::collections::BTreeMap;
#[test]
fn unreadable_encoding_has_a_file_diagnostic_and_is_not_missing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("invalid.rs"), [0xff, 0xfe]).unwrap();
    let project = Project::new(dir.path()).unwrap();
    let inventory = scan(&project, &[], &OperationContext::default()).unwrap();
    assert!(matches!(
        inventory.observations["invalid.rs"],
        Observation::Unchecked
    ));
    assert_eq!(inventory.file_diagnostics[0]["path"], "invalid.rs");
    assert_eq!(inventory.file_diagnostics[0]["code"], "SYNC_UNCHECKED");
}
#[test]
fn restricted_policy_counts_do_not_read_or_disclose_paths() {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::new(dir.path()).unwrap();
    let tracked: Vec<String> = [
        "NUXT.CONFIG.ts",
        "vite.config.ts",
        ".ENV/private.rs",
        ".git/internal.rs",
        "key.pem",
        "appsettings.production.json",
        ".npmrc",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let inventory = scan(&project, &tracked, &OperationContext::default()).unwrap();
    assert_eq!(inventory.restricted_indexed_files, 7);
    assert!(inventory.observations.is_empty());
    let value = serde_json::to_value(inventory).unwrap();
    assert_eq!(value["restrictedIndexedReasons"]["config_filename"], 2);
    for reason in [
        "sensitive_path_prefix",
        "version_control_directory",
        "private_key_extension",
        "appsettings_filename",
        "npmrc_filename",
    ] {
        assert_eq!(value["restrictedIndexedReasons"][reason], 1);
    }
    for path in tracked {
        assert!(!value.to_string().contains(&path));
    }
    assert!(!restricted("src/profile.ts"));
    assert!(!restricted("src/config.ts"));
    assert!(restricted("SRC\\NUXT.CONFIG.TS"));
}
#[test]
fn aggregate_budget_never_turns_an_existing_source_into_a_deletion() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("source.rs"), "fn source() {}").unwrap();
    let project = Project::new(dir.path()).unwrap();
    let mut inventory = Inventory {
        observations: BTreeMap::new(),
        discovery_complete: true,
        visited_entries: 0,
        reserved_source_bytes: MAX_BYTES,
        skipped_entries: 0,
        restricted_indexed_files: 0,
        restricted_indexed_reasons: BTreeMap::new(),
        budget_exhausted: false,
        scope_excluded: Vec::new(),
        scope: serde_json::Value::Null,
        file_diagnostics: Vec::new(),
    };
    let observed = harness_application::indexing::inventory::observe(
        &mut inventory,
        &NativeInventory {
            project: &project,
            scope: crate::adapters::index_store::scope::Snapshot::load(
                &project,
                &OperationContext::default(),
            )
            .unwrap(),
        },
        "source.rs",
        &OperationContext::default(),
    )
    .unwrap();
    assert!(matches!(observed, Observation::Unchecked));
    assert!(inventory.budget_exhausted);
    assert!(!inventory.discovery_complete);
    assert_eq!(inventory.reserved_source_bytes, MAX_BYTES);
}

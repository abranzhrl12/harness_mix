use super::*;

fn edge(target: &str, metadata: Option<&str>) -> Incoming {
    Incoming {
        source: "caller".into(),
        target: target.into(),
        kind: "calls".into(),
        line: Some(3),
        col: Some(4),
        metadata: metadata.map(str::to_owned),
        provenance: None,
        path: "caller.rs".into(),
        language: "rust".into(),
    }
}

#[test]
fn incoming_plan_retains_provider_and_remaps_stable_identity() {
    let provider_ids = BTreeSet::from(["provider-node".to_owned()]);
    let paired = BTreeMap::from([("old".to_owned(), "new".to_owned())]);
    let provider =
        incoming_disposition(edge("provider-node", None), &provider_ids, &paired).unwrap();
    assert!(matches!(provider, IncomingDisposition::Retain(_)));
    let remapped = incoming_disposition(edge("old", None), &provider_ids, &paired).unwrap();
    let IncomingDisposition::Retain(remapped) = remapped else {
        panic!("identity match must retain the edge")
    };
    assert_eq!(remapped.target, "new");
}

#[test]
fn incoming_plan_recovers_original_reference_when_identity_disappears() {
    let disposition = incoming_disposition(
        edge("old", Some(r#"{"refName":"crate::old","refKind":"calls"}"#)),
        &BTreeSet::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    let IncomingDisposition::Recover { name, kind, .. } = disposition else {
        panic!("missing identity must recover its unresolved reference")
    };
    assert_eq!(name, "crate::old");
    assert_eq!(kind, "calls");
}

#[test]
fn unchanged_plan_obeys_hash_errors_configuration_and_force() {
    let clean = ("same".to_owned(), Some("[]".to_owned()));
    let digest = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    assert!(is_unchanged(
        Some(&clean),
        Some(b"same"),
        false,
        false,
        digest
    ));
    assert!(!is_unchanged(
        Some(&clean),
        Some(b"changed"),
        false,
        false,
        digest
    ));
    assert!(!is_unchanged(
        Some(&clean),
        Some(b"same"),
        true,
        false,
        digest
    ));
    assert!(!is_unchanged(
        Some(&clean),
        Some(b"same"),
        false,
        true,
        digest
    ));
    let failed = ("same".to_owned(), Some("not-json".to_owned()));
    assert!(!is_unchanged(
        Some(&failed),
        Some(b"same"),
        false,
        false,
        digest
    ));
    assert!(is_unchanged(None, None, false, false, digest));
}

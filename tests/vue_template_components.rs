#![cfg(windows)]
use agent_harness::adapters::index_store::{sync_file, sync_file_with_tsconfig};
use harness_domain::execution::OperationContext;
use rusqlite::{Connection, params};
use std::{fs, path::Path};

fn fixture(root: &Path) {
    fs::create_dir_all(root.join(".codegraph")).unwrap();
    let db = Connection::open(root.join(".codegraph/codegraph.db")).unwrap();
    db.execute_batch(include_str!("../vendor/codegraph-native/schema.sql"))
        .unwrap();
    db.execute(
        "INSERT INTO schema_versions(version,applied_at) VALUES(11,0)",
        [],
    )
    .unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL").unwrap();
}

fn write(root: &Path, path: &str, source: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
}

fn sync(root: &Path, path: &str) {
    sync_file(root, path, &OperationContext::default()).unwrap();
}

fn sync_with_tsconfig(root: &Path, path: &str) {
    sync_file_with_tsconfig(root, path, "tsconfig.json", &OperationContext::default()).unwrap();
}

fn component_edges(root: &Path, source: &str, target: &str) -> i64 {
    Connection::open(root.join(".codegraph/codegraph.db"))
        .unwrap()
        .query_row(
            "SELECT count(*) FROM edges e JOIN nodes s ON s.id=e.source JOIN nodes t ON t.id=e.target WHERE e.kind='references' AND s.file_path=?1 AND t.file_path=?2 AND t.kind='component' AND json_extract(e.metadata,'$.resolvedBy')='vue-script-setup-component-import-subset-v1'",
            params![source, target],
            |row| row.get(0),
        )
        .unwrap()
}

fn pending_components(root: &Path, source: &str, name: &str) -> i64 {
    Connection::open(root.join(".codegraph/codegraph.db"))
        .unwrap()
        .query_row(
            "SELECT count(*) FROM unresolved_refs WHERE file_path=?1 AND reference_name=?2 AND reference_kind='vue_template_component' AND status='pending'",
            params![source, name],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn setup_default_imports_resolve_pascal_kebab_alias_and_tsconfig_paths() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    write(
        root.path(),
        "tsconfig.json",
        r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["*"]}}}"#,
    );
    write(
        root.path(),
        "components/DefaultCard.vue",
        "<template><section/></template>",
    );
    write(
        root.path(),
        "components/LocalCard.vue",
        "<template><article/></template>",
    );
    sync(root.path(), "components/DefaultCard.vue");
    sync(root.path(), "components/LocalCard.vue");
    write(
        root.path(),
        "App.vue",
        "<script setup lang=\"ts\">\nimport DefaultCard from './components/DefaultCard.vue';\nimport LocalAlias from '@/components/LocalCard';\n</script>\n<template><DefaultCard/><local-alias/></template>",
    );
    sync_with_tsconfig(root.path(), "App.vue");

    assert_eq!(
        component_edges(root.path(), "App.vue", "components/DefaultCard.vue"),
        1
    );
    assert_eq!(
        component_edges(root.path(), "App.vue", "components/LocalCard.vue"),
        1
    );
    assert_eq!(pending_components(root.path(), "App.vue", "DefaultCard"), 0);
    assert_eq!(pending_components(root.path(), "App.vue", "local-alias"), 0);
}

#[test]
fn missing_component_target_retries_when_the_vue_file_is_indexed() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    write(
        root.path(),
        "App.vue",
        "<script setup>import MissingView from './MissingView.vue';</script><template><MissingView/></template>",
    );
    sync(root.path(), "App.vue");
    assert_eq!(pending_components(root.path(), "App.vue", "MissingView"), 1);
    assert_eq!(
        component_edges(root.path(), "App.vue", "MissingView.vue"),
        0
    );

    write(
        root.path(),
        "MissingView.vue",
        "<template><main/></template>",
    );
    sync(root.path(), "MissingView.vue");
    assert_eq!(
        component_edges(root.path(), "App.vue", "MissingView.vue"),
        1
    );
    assert_eq!(pending_components(root.path(), "App.vue", "MissingView"), 0);
}

#[test]
fn ambiguous_normalized_import_bindings_remain_pending() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    write(root.path(), "first.vue", "<template><main/></template>");
    write(root.path(), "second.vue", "<template><aside/></template>");
    sync(root.path(), "first.vue");
    sync(root.path(), "second.vue");
    write(
        root.path(),
        "App.vue",
        "<script setup>import CardView from './first.vue'; import cardView from './second.vue';</script><template><card-view/></template>",
    );
    sync(root.path(), "App.vue");

    assert_eq!(component_edges(root.path(), "App.vue", "first.vue"), 0);
    assert_eq!(component_edges(root.path(), "App.vue", "second.vue"), 0);
    assert_eq!(pending_components(root.path(), "App.vue", "card-view"), 1);
}

#[test]
fn named_exports_from_sfc_are_not_assumed_to_be_its_default_component() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    write(root.path(), "Card.vue", "<template><main/></template>");
    sync(root.path(), "Card.vue");
    write(
        root.path(),
        "App.vue",
        "<script setup>import { Card as LocalCard } from './Card.vue';</script><template><LocalCard/></template>",
    );
    sync(root.path(), "App.vue");

    assert_eq!(component_edges(root.path(), "App.vue", "Card.vue"), 0);
    assert_eq!(pending_components(root.path(), "App.vue", "LocalCard"), 1);
}

#[test]
fn stale_parent_source_is_not_used_to_publish_a_component_edge() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    write(
        root.path(),
        "App.vue",
        "<script setup>import OldCard from './OldCard.vue';</script><template><OldCard/></template>",
    );
    sync(root.path(), "App.vue");
    write(
        root.path(),
        "App.vue",
        "<script setup>import NewCard from './NewCard.vue';</script><template><NewCard/></template>",
    );
    write(root.path(), "OldCard.vue", "<template><main/></template>");
    sync(root.path(), "OldCard.vue");

    assert_eq!(component_edges(root.path(), "App.vue", "OldCard.vue"), 0);
    assert_eq!(pending_components(root.path(), "App.vue", "OldCard"), 1);

    write(
        root.path(),
        "NewCard.vue",
        "<template><section/></template>",
    );
    sync(root.path(), "NewCard.vue");
    assert_eq!(component_edges(root.path(), "App.vue", "NewCard.vue"), 0);
    sync(root.path(), "App.vue");
    assert_eq!(component_edges(root.path(), "App.vue", "NewCard.vue"), 1);
    assert_eq!(pending_components(root.path(), "App.vue", "NewCard"), 0);
}

#[test]
fn stale_component_target_stays_pending_until_target_is_reindexed() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path());
    write(root.path(), "Card.vue", "<template><main/></template>");
    sync(root.path(), "Card.vue");
    write(root.path(), "Card.vue", "<template><section/></template>");
    write(
        root.path(),
        "App.vue",
        "<script setup>import Card from './Card.vue';</script><template><Card/></template>",
    );
    sync(root.path(), "App.vue");

    assert_eq!(component_edges(root.path(), "App.vue", "Card.vue"), 0);
    assert_eq!(pending_components(root.path(), "App.vue", "Card"), 1);
    sync(root.path(), "Card.vue");
    assert_eq!(component_edges(root.path(), "App.vue", "Card.vue"), 1);
}

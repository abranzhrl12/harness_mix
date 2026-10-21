use super::*;

fn row(kind: &str, schema: &str, name: &str, d1: &str, d2: &str, d3: &str, d4: &str) -> MapRow {
    MapRow::new(kind, schema, name, d1, d2, d3, d4)
}

#[test]
fn assemble_attaches_primary_keys_and_foreign_keys() {
    let rows = vec![
        row("relation", "public", "orders", "table", "", "", ""),
        row("primaryKey", "public", "orders", "id", "", "", ""),
        row(
            "foreignKey",
            "public",
            "orders_user_fk",
            "orders",
            "user_id",
            "users",
            "id",
        ),
    ];
    let map = assemble("postgresql", rows, 50);
    assert_eq!(map["relations"][0]["primaryKey"][0], "id");
    assert_eq!(map["foreignKeys"][0]["child"], "orders");
    assert_eq!(map["foreignKeys"][0]["parent"], "users");
    assert_eq!(map["foreignKeys"][0]["parentColumns"][0], "id");
}

#[test]
fn assemble_reports_truncation_when_the_relation_limit_is_reached() {
    let rows = vec![
        row("relation", "public", "a", "table", "", "", ""),
        row("relation", "public", "b", "table", "", "", ""),
    ];
    let map = assemble("postgresql", rows, 1);
    assert_eq!(map["counts"]["relations"], 1);
    assert_eq!(map["truncated"], true);
}

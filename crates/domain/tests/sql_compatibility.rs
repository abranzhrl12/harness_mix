use harness_domain::sql::assert_sql_read_only;

#[test]
fn public_read_only_policy_keeps_comments_quotes_and_statement_rules() {
    assert!(assert_sql_read_only("SELECT 'DROP' -- UPDATE\nFROM [table];").is_ok());
    assert!(assert_sql_read_only("WITH c AS (SELECT 1) SELECT * FROM c").is_ok());
    for sql in [
        "",
        "UPDATE dbo.Item SET Name = 'x'",
        "SELECT 1; SELECT 2",
        "SELECT (1",
        "SELECT 'unterminated",
        "DELETE FROM dbo.Item",
    ] {
        assert!(assert_sql_read_only(sql).is_err(), "accepted {sql:?}");
    }
}

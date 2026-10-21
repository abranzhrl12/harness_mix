use super::status::parse_status;

#[test]
fn parses_branch_spaces_conflict_and_rename_with_nul_delimiters() {
    let output = concat!(
        "# branch.oid (initial)\0# branch.head main\0# branch.ab +2 -3\0? a b\0u UU N... 100644 100644 100644 100644 a b c conflict\0",
        "2 R. N... 100644 100644 100644 a b R100 new name\0old name\0"
    );
    let result = parse_status(output).unwrap();
    assert_eq!(result["unborn"], true);
    assert_eq!(result["ahead"], 2);
    assert_eq!(result["behind"], 3);
    assert_eq!(result["entries"][0]["path"], "a b");
    assert_eq!(result["entries"][1]["kind"], "conflict");
    assert_eq!(result["entries"][2]["from"], "old name");
    assert!(parse_status("1 broken\0").is_err());
    assert!(parse_status("? truncated").is_err());
}

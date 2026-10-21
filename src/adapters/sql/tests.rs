use super::{
    connection::block_on_safe,
    diagnostics::{
        operation_failed, operation_failed_with_server_diagnostic, safe_schema_message,
        server_diagnostic, sql_error,
    },
    operations::{
        ObjectResolution, ParsedSqlObjectName, ROUTINE_PARAMETERS_SQL, ResolvedSqlObject,
        classify_object_candidates, escape_like, inject_top, object_lookup_parameters,
        object_resolution_failure, parse_sql_object_name, quote_sql_identifier,
        routine_definition_diagnostic, routine_parameter_limit,
    },
};
use serde_json::json;
use std::time::Duration;
#[test]
fn literal_like_wildcards_are_escaped_without_replacement_templates() {
    assert_eq!(escape_like("a%_~[é"), "a~%~_~~~[é");
}
#[test]
fn top_is_injected_only_when_a_plain_select_lacks_one() {
    for (sql, expected) in [
        ("SELECT 1", "SELECT TOP (50) 1"),
        ("select*from t", "select TOP (50)*from t"),
        (
            "select distinct name from t",
            "select distinct TOP (50) name from t",
        ),
        (
            "-- why\r\nselect /* c */ * from t",
            "-- why\r\nselect TOP (50) /* c */ * from t",
        ),
        ("select top 10 * from t", "select top 10 * from t"),
        ("select TOP (10) * from t", "select TOP (10) * from t"),
        (
            "with c as (select 1) select * from c",
            "with c as (select 1) select * from c",
        ),
        ("update t set a = 1", "update t set a = 1"),
    ] {
        assert_eq!(inject_top(sql, 50), expected, "{sql}");
    }
}

#[test]
fn missing_routine_definition_has_a_permission_or_schema_diagnostic() {
    let diagnostic = routine_definition_diagnostic(None);
    assert_eq!(diagnostic[0]["code"], "SQL_DEFINITION_UNAVAILABLE");
    assert_eq!(
        diagnostic[0]["message"],
        "The routine is visible in the catalog, but its definition is unavailable. VIEW DEFINITION permission may be missing, the module may be encrypted, or SQL Server may have another metadata visibility restriction; this result does not identify the cause."
    );
    assert!(
        routine_definition_diagnostic(Some("CREATE PROCEDURE dbo.sample AS SELECT 1")).is_empty()
    );
}

#[test]
fn routine_parameter_catalog_query_is_object_scoped_ordered_and_keeps_return_parameters() {
    assert!(ROUTINE_PARAMETERS_SQL.contains("WHERE p.object_id = @P1"));
    assert!(ROUTINE_PARAMETERS_SQL.contains("ORDER BY p.parameter_id"));
    assert!(ROUTINE_PARAMETERS_SQL.contains("p.is_output"));
    assert!(ROUTINE_PARAMETERS_SQL.contains("p.parameter_id"));
    assert!(!ROUTINE_PARAMETERS_SQL.contains("p.parameter_id > 0"));
}

#[test]
fn routine_parameter_limit_includes_a_bounded_overflow_sentinel() {
    assert_eq!(routine_parameter_limit(0), 1);
    assert_eq!(routine_parameter_limit(10), 11);
    assert_eq!(routine_parameter_limit(usize::MAX), 2101);
}

#[test]
fn sql_object_name_parser_handles_delimited_and_escaped_identifiers() {
    assert!(matches!(
        parse_sql_object_name("ORDEN_SERVICIO"),
        Ok(ParsedSqlObjectName::Unqualified(name)) if name == "ORDEN_SERVICIO"
    ));
    assert!(matches!(
        parse_sql_object_name("[taller].[ORDEN_SERVICIO]"),
        Ok(ParsedSqlObjectName::Qualified(schema, name))
            if schema == "taller" && name == "ORDEN_SERVICIO"
    ));
    assert!(matches!(
        parse_sql_object_name("[schema.with.dot].[order]]detail]"),
        Ok(ParsedSqlObjectName::Qualified(schema, name))
            if schema == "schema.with.dot" && name == "order]detail"
    ));
    assert!(matches!(
        parse_sql_object_name("[object.with.dot]"),
        Ok(ParsedSqlObjectName::Unqualified(name)) if name == "object.with.dot"
    ));
}

#[test]
fn explicit_schema_is_carried_verbatim_and_unqualified_names_search_all_schemas() {
    assert_eq!(
        object_lookup_parameters(&ParsedSqlObjectName::Qualified(
            "taller".into(),
            "ORDEN_SERVICIO".into()
        )),
        ("taller".into(), "ORDEN_SERVICIO".into())
    );
    assert_eq!(
        object_lookup_parameters(&ParsedSqlObjectName::Unqualified("ORDEN_SERVICIO".into())),
        (String::new(), "ORDEN_SERVICIO".into())
    );
}

#[test]
fn sql_object_name_parser_rejects_unsupported_or_malformed_names() {
    for name in [
        "db.schema.table",
        "[schema].[table",
        "schema..table",
        "schema.table.extra",
        "[bad\nname]",
    ] {
        assert!(parse_sql_object_name(name).is_err(), "{name}");
    }
    assert!(parse_sql_object_name(&format!("[{}]", "x".repeat(129))).is_err());
}

#[test]
fn resolved_sql_identifiers_are_always_reusable_and_delimited() {
    assert_eq!(quote_sql_identifier("taller"), "[taller]");
    assert_eq!(quote_sql_identifier("order.detail"), "[order.detail]");
    assert_eq!(quote_sql_identifier("order]detail"), "[order]]detail]");
}

#[test]
fn ambiguous_sql_object_response_bounds_and_marks_candidates() {
    let response = object_resolution_failure(
        "SQL_OBJECT_AMBIGUOUS",
        "Object name matches multiple visible schemas; specify the schema",
        &["dbo.ORDEN_SERVICIO".into(), "taller.ORDEN_SERVICIO".into()],
        false,
    );
    assert_eq!(response["diagnostics"][0]["code"], "SQL_OBJECT_AMBIGUOUS");
    assert_eq!(
        response["diagnostics"][0]["candidates"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(response["diagnostics"][0]["candidatesTruncated"], false);
}

#[test]
fn catalog_candidates_resolve_unique_or_report_bounded_ambiguity() {
    let candidate = |schema: &str| ResolvedSqlObject {
        schema: schema.into(),
        name: "ORDEN_SERVICIO".into(),
        object_id: 1,
        kind: "USER_TABLE".into(),
    };
    assert!(matches!(
        classify_object_candidates(Vec::new()),
        ObjectResolution::NotFound
    ));
    assert!(matches!(
        classify_object_candidates(vec![candidate("taller")]),
        ObjectResolution::Unique(value) if value.schema == "taller"
    ));
    let many = (0..6).map(|n| candidate(&format!("schema{n}"))).collect();
    assert!(matches!(
        classify_object_candidates(many),
        ObjectResolution::Ambiguous { candidates, truncated: true } if candidates.len() == 5
    ));
}

#[test]
fn driver_error_text_is_never_exposed() {
    assert_eq!(
        sql_error(tiberius::error::Error::Tls(
            "private connection details".into()
        ))
        .0,
        "SQL_TLS"
    );
    let failure = sql_error(tiberius::error::Error::Protocol(
        "SELECT private value".into(),
    ));
    assert!(!failure.1.contains("private"));
    let output = operation_failed(("TIMEOUT", "operation deadline exceeded"));
    assert_eq!(output["timedOut"], true);
    assert_eq!(output["serverCancellationConfirmed"], false);
}
#[test]
fn schema_errors_echo_only_allowlisted_identifiers() {
    assert_eq!(
        safe_schema_message(207, "Invalid column name 'tEmpresaRuc'.").as_deref(),
        Some("SQL Server 207: Invalid column name 'tEmpresaRuc'.")
    );
    assert!(safe_schema_message(208, "Invalid object name 'dbo.MEMPRESA'.").is_some());
    assert!(
        safe_schema_message(
            4104,
            "The multi-part identifier \"a.Id\" could not be bound."
        )
        .is_some()
    );
    for message in [
        "Invalid column name 'Password=secret'.",
        "Invalid column name 'valid'.\nUser ID=sa;Password=secret",
        "Invalid column name 'abc\nsecret'.",
        "Invalid column name 'SELECT private value'.",
        "Invalid column name 'x';Password=secret'.",
        "Invalid column name ''.",
    ] {
        assert!(safe_schema_message(207, message).is_none());
    }
    assert!(
        safe_schema_message(207, &format!("Invalid column name '{}'.", "x".repeat(257))).is_none()
    );
    assert!(safe_schema_message(18456, "Login failed for user 'secret'.").is_none());
    assert!(
        safe_schema_message(245, "Conversion failed when converting 'private value'.").is_none()
    );
    assert!(safe_schema_message(50000, "Invalid column name 'secret'.").is_none());
    assert!(
        server_diagnostic(&tiberius::error::Error::Protocol("Password=secret".into())).is_none()
    );
}
#[test]
fn invalid_column_is_visible_in_the_public_failure_without_raw_server_text() {
    for (raw, expected) in [
        (
            "Invalid column name 'tEmpresaRuc'.",
            "SQL Server 207: Invalid column name 'tEmpresaRuc'.",
        ),
        (
            "Invalid column name 'x';Password=secret'.",
            "SQL Server 207: Invalid column name",
        ),
    ] {
        let diagnostic = json!({
            "number": 207,
            "message": safe_schema_message(207, raw),
            "messageSanitized": true
        });
        let output = operation_failed_with_server_diagnostic(
            ("SQL_INVALID_COLUMN", "SQL Server 207: Invalid column name"),
            Some(&diagnostic),
        );
        assert_eq!(output["diagnostics"][0]["message"], expected);
        assert_eq!(output["diagnostics"][0]["code"], "SQL_INVALID_COLUMN");
        assert!(!output.to_string().contains("Password=secret"));
    }
}
#[tokio::test]
async fn sync_bridge_handles_current_thread_runtime() {
    assert_eq!(
        block_on_safe(async {
            tokio::time::sleep(Duration::from_millis(1)).await;
            42
        }),
        42
    );
}

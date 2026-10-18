//! Translate named placeholders, never parameter values. Tiberius binds values via RPC.
use harness_domain::Failure;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};

const INVALID: Failure = (
    "SQL_PARAMETERS",
    "Use at most 128 named scalar parameters; every placeholder must be bound and every supplied parameter used",
);

pub(super) fn prepare(
    sql: &str,
    parameters: Option<&Map<String, Value>>,
) -> Result<(String, Vec<Value>), Failure> {
    if sql.len() > 65536 {
        return Err(INVALID);
    }
    let mut supplied = BTreeMap::new();
    let mut bytes = 0;
    for (name, value) in parameters.into_iter().flatten() {
        let name = name.strip_prefix('@').unwrap_or(name);
        if name.is_empty()
            || name.len() > 128
            || !name
                .bytes()
                .enumerate()
                .all(|(i, c)| c.is_ascii_alphabetic() || c == b'_' || (i > 0 && c.is_ascii_digit()))
        {
            return Err(INVALID);
        }
        if value.is_array() || value.is_object() {
            return Err(INVALID);
        }
        if let Value::Number(n) = value {
            // Reject numbers outside Decimal's exact range, rather than silently rounding to f64.
            rust_decimal::Decimal::from_str_exact(&n.to_string()).map_err(|_| INVALID)?;
        }
        bytes += value.to_string().len();
        if bytes > 65536
            || supplied
                .insert(name.to_ascii_lowercase(), value.clone())
                .is_some()
        {
            return Err(INVALID);
        }
    }
    if supplied.len() > 128 {
        return Err(INVALID);
    }
    let source = sql.as_bytes();
    let mut out = Vec::with_capacity(source.len());
    let mut ordered = Vec::new();
    let mut seen = HashMap::new();
    let mut i = 0;
    while i < source.len() {
        let start = i;
        match source[i] {
            b'\'' | b'"' | b'[' => {
                let close = if source[i] == b'[' { b']' } else { source[i] };
                i += 1;
                loop {
                    if i >= source.len() {
                        return Err(INVALID);
                    }
                    if source[i] == close {
                        i += 1;
                        if source.get(i) == Some(&close) {
                            i += 1;
                        } else {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            b'-' if source.get(i + 1) == Some(&b'-') => {
                while i < source.len() && source[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if source.get(i + 1) == Some(&b'*') => {
                let mut depth = 1;
                i += 2;
                while depth > 0 {
                    if i + 1 >= source.len() {
                        return Err(INVALID);
                    }
                    if &source[i..i + 2] == b"/*" {
                        depth += 1;
                        i += 2;
                    } else if &source[i..i + 2] == b"*/" {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            }
            b'@' => {
                i += 1;
                if source.get(i) == Some(&b'@') {
                    i += 1;
                    while i < source.len()
                        && (source[i].is_ascii_alphanumeric() || source[i] == b'_')
                    {
                        i += 1;
                    }
                } else {
                    while i < source.len()
                        && (source[i].is_ascii_alphanumeric() || source[i] == b'_')
                    {
                        i += 1;
                    }
                    let key = sql[start + 1..i].to_ascii_lowercase();
                    let value = supplied.get(&key).ok_or(INVALID)?;
                    let next = seen.len() + 1;
                    let index = *seen.entry(key).or_insert_with(|| {
                        ordered.push(value.clone());
                        next
                    });
                    out.extend_from_slice(format!("@P{index}").as_bytes());
                    continue;
                }
            }
            _ => i += 1,
        }
        out.extend_from_slice(&source[start..i]);
    }
    if seen.len() != supplied.len() {
        return Err(INVALID);
    }
    Ok((String::from_utf8(out).map_err(|_| INVALID)?, ordered))
}

pub(super) fn bind(query: &mut tiberius::Query<'_>, value: Value) {
    match value {
        Value::Null => query.bind(None::<String>),
        Value::Bool(v) => query.bind(v),
        Value::String(v) => query.bind(v),
        Value::Number(v) => {
            if let Some(v) = v.as_i64() {
                query.bind(v);
            } else {
                query.bind(
                    rust_decimal::Decimal::from_str_exact(&v.to_string())
                        .expect("validated decimal"),
                );
            }
        }
        _ => unreachable!("validated scalar"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn placeholders_are_ordered_reused_and_values_never_interpolated() {
        let values = json!({"id":7,"@label":"'; DROP TABLE t;--"});
        let (sql, bound) = prepare("SELECT @label, @id, @LABEL, '@id', [@id], \"@id\", @@VERSION /* @absent /* @also */ */ -- @absent\n", values.as_object()).unwrap();
        assert_eq!(bound, vec![values["@label"].clone(), json!(7)]);
        assert!(sql.starts_with("SELECT @P1, @P2, @P1, '@id', [@id], \"@id\", @@VERSION"));
        assert!(!sql.contains("DROP"));
    }
    #[test]
    fn rejects_missing_unused_ambiguous_and_nonscalar_values() {
        for (sql, values) in [
            ("SELECT @x", json!({})),
            ("SELECT 1", json!({"x":1})),
            ("SELECT @x", json!({"x":[],})),
            ("SELECT @x", json!({"x":1,"@X":2})),
            ("SELECT @x", json!({"x":1e100})),
        ] {
            assert_eq!(
                prepare(sql, values.as_object()).unwrap_err().0,
                "SQL_PARAMETERS"
            );
        }
    }
    #[test]
    fn all_supported_scalars_are_bound_via_driver() {
        let values = serde_json::json!({"a":null,"b":true,"c":"text","d":42,"e":12.25});
        let (sql, values) = prepare("SELECT @a,@b,@c,@d,@e", values.as_object()).unwrap();
        let mut query = tiberius::Query::new(sql);
        for value in values {
            bind(&mut query, value);
        }
        assert_eq!(query.param_count(), 5);
    }
}

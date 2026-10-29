use harness_domain::{Failure, browser::BrowserArgs};
use url::Url;

pub(crate) const MAX_EVENTS: usize = 256;
pub(crate) const MAX_EVENT_MESSAGE: usize = 2_048;
pub(crate) const MAX_HTML_CHARS: usize = 100_000;
pub(crate) const STANDARD_CDP_PORTS: [u16; 4] = [9222, 9229, 9223, 9224];

const MAX_ARGUMENT_BYTES: usize = 256;
const MAX_ENDPOINT_BYTES: usize = 4_096;
const MAX_SELECTOR_BYTES: usize = 4_096;

fn bounded(value: &str, max: usize, field: &'static str) -> Result<(), Failure> {
    if value.len() <= max {
        Ok(())
    } else {
        Err(("ARGUMENT", field))
    }
}

fn validate_session(session: Option<&str>) -> Result<(), Failure> {
    let value = session
        .filter(|value| !value.trim().is_empty())
        .ok_or(("BROWSER_SESSION", "La sesión Harness es obligatoria"))?;
    bounded(value, MAX_ARGUMENT_BYTES, "La sesión excede 256 bytes")
}

fn validate_ids(connection_id: &str, target_id: Option<&str>) -> Result<(), Failure> {
    bounded(
        connection_id,
        MAX_ARGUMENT_BYTES,
        "El ID de conexión excede 256 bytes",
    )?;
    if let Some(target_id) = target_id {
        bounded(
            target_id,
            MAX_ARGUMENT_BYTES,
            "El ID de pestaña excede 256 bytes",
        )?;
    }
    Ok(())
}

/// Validates and returns the supplied Harness session identity unchanged.
pub(crate) fn required_session(session: Option<String>) -> Result<String, Failure> {
    let session = session.ok_or(("BROWSER_SESSION", "La sesión Harness es obligatoria"))?;
    validate_session(Some(&session))?;
    Ok(session)
}

/// Ensures a connection is used by the Harness session that created it.
pub(crate) fn validate_session_owner(owner: &str, session: &str) -> Result<(), Failure> {
    if owner == session {
        Ok(())
    } else {
        Err((
            "BROWSER_SESSION",
            "La conexión pertenece a otra sesión Harness",
        ))
    }
}

/// Returns true when the endpoint string requests automatic local CDP discovery.
pub(crate) fn is_auto_endpoint(endpoint: &str) -> bool {
    let trimmed = endpoint.trim();
    trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("auto")
        || trimmed.eq_ignore_ascii_case("localhost")
        || trimmed.eq_ignore_ascii_case("127.0.0.1")
        || trimmed.eq_ignore_ascii_case("[::1]")
        || trimmed.eq_ignore_ascii_case("::1")
        || trimmed.eq_ignore_ascii_case("http://localhost")
        || trimmed.eq_ignore_ascii_case("http://localhost/")
        || trimmed.eq_ignore_ascii_case("http://127.0.0.1")
        || trimmed.eq_ignore_ascii_case("http://127.0.0.1/")
        || trimmed.eq_ignore_ascii_case("http://[::1]")
        || trimmed.eq_ignore_ascii_case("http://[::1]/")
}

/// Splits a user-provided `endpoint` and optional `tab` filter into:
/// `(cdp_endpoint_or_auto, effective_tab_filter)`.
///
/// If the caller passed a localhost application URL/port as `endpoint` (e.g.
/// `http://localhost:5173/app` or `:3000`) instead of a CDP debugging port,
/// this treats CDP discovery as `"auto"` and uses that URL/port as the tab filter
/// (unless an explicit `tab` filter was already provided).
pub(crate) fn parse_connect_input(
    endpoint: &str,
    tab: Option<&str>,
) -> Result<(Option<String>, Option<String>), Failure> {
    let explicit_tab = tab
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let trimmed = endpoint.trim();
    if is_auto_endpoint(trimmed) {
        return Ok((None, explicit_tab));
    }

    // Check if shorthand port `:9222` or `:3000`
    if let Some(port_str) = trimmed.strip_prefix(':') {
        if let Ok(port) = port_str.parse::<u16>() {
            if port > 0 {
                if STANDARD_CDP_PORTS.contains(&port) {
                    return Ok((Some(format!("http://127.0.0.1:{port}")), explicit_tab));
                }
                return Ok((
                    None,
                    Some(explicit_tab.unwrap_or_else(|| trimmed.to_owned())),
                ));
            }
        }
    }
    if let Ok(port) = trimmed.parse::<u16>() {
        if port > 0 {
            if STANDARD_CDP_PORTS.contains(&port) {
                return Ok((Some(format!("http://127.0.0.1:{port}")), explicit_tab));
            }
            return Ok((
                None,
                Some(explicit_tab.unwrap_or_else(|| format!(":{port}"))),
            ));
        }
    }

    // Bare loopback host:port like `localhost:3000` or `127.0.0.1:9222`
    if !trimmed.contains("://") {
        if let Ok(url) = Url::parse(&format!("http://{trimmed}")) {
            let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
            if matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1" | "[::1]")
                && url.username().is_empty()
                && url.password().is_none()
            {
                if let Some(port) = url.port() {
                    if STANDARD_CDP_PORTS.contains(&port)
                        && matches!(
                            url.path(),
                            "" | "/" | "/json/version" | "/json" | "/json/list"
                        )
                        && url.query().is_none()
                        && url.fragment().is_none()
                    {
                        return Ok((Some(format!("http://{host}:{port}")), explicit_tab));
                    }
                    // Localhost app URL/port shorthand like `localhost:5173/dashboard`
                    return Ok((
                        None,
                        Some(explicit_tab.unwrap_or_else(|| trimmed.to_owned())),
                    ));
                }
            }
        }
    }

    // Standard direct ws:// or http:// URL
    let url = Url::parse(trimmed)
        .map_err(|_| ("BROWSER_ENDPOINT", "El endpoint CDP no es una URL válida"))?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
    {
        return Err((
            "BROWSER_ENDPOINT",
            "El endpoint CDP no puede incluir credenciales, fragmentos ni query",
        ));
    }

    match url.scheme() {
        "ws" => {
            validate_endpoint(trimmed)?;
            Ok((Some(trimmed.to_owned()), explicit_tab))
        }
        "http" => {
            let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
            if !matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1" | "[::1]") {
                return Err((
                    "BROWSER_ENDPOINT",
                    "Solo se permiten endpoints CDP HTTP loopback",
                ));
            }
            let Some(_port) = url.port().filter(|&port| port != 0) else {
                return Err((
                    "BROWSER_ENDPOINT",
                    "Solo se permiten endpoints CDP HTTP loopback",
                ));
            };
            if matches!(url.path(), "" | "/" | "/json/version") {
                Ok((Some(trimmed.to_owned()), explicit_tab))
            } else {
                Err((
                    "BROWSER_ENDPOINT",
                    "Solo se permiten endpoints CDP HTTP loopback",
                ))
            }
        }
        _ => Err((
            "BROWSER_ENDPOINT",
            "Use un endpoint ws:// directo o un HTTP loopback para descubrir CDP",
        )),
    }
}

/// Enforces the adapter's request bounds before any actor or browser work starts.
pub(crate) fn validate_request(args: &BrowserArgs) -> Result<(), Failure> {
    match args {
        BrowserArgs::Connect {
            endpoint,
            tab,
            session,
        } => {
            validate_session(session.as_deref())?;
            bounded(
                endpoint,
                MAX_ENDPOINT_BYTES,
                "El endpoint excede 4096 bytes",
            )?;
            if let Some(tab) = tab {
                bounded(
                    tab,
                    MAX_ARGUMENT_BYTES,
                    "El filtro de pestaña excede 256 bytes",
                )?;
            }
            validate_connect_endpoint(endpoint)?;
        }
        BrowserArgs::Discover {
            endpoint,
            tab,
            session,
        } => {
            validate_session(session.as_deref())?;
            if let Some(endpoint) = endpoint {
                bounded(
                    endpoint,
                    MAX_ENDPOINT_BYTES,
                    "El endpoint excede 4096 bytes",
                )?;
                validate_connect_endpoint(endpoint)?;
            }
            if let Some(tab) = tab {
                bounded(
                    tab,
                    MAX_ARGUMENT_BYTES,
                    "El filtro de pestaña excede 256 bytes",
                )?;
            }
        }
        BrowserArgs::Targets {
            connection_id,
            tab,
            session,
        } => {
            validate_session(session.as_deref())?;
            validate_ids(connection_id, None)?;
            if let Some(tab) = tab {
                bounded(
                    tab,
                    MAX_ARGUMENT_BYTES,
                    "El filtro de pestaña excede 256 bytes",
                )?;
            }
        }
        BrowserArgs::Disconnect {
            connection_id,
            session,
        } => {
            validate_session(session.as_deref())?;
            validate_ids(connection_id, None)?;
        }
        BrowserArgs::Attach {
            connection_id,
            target_id,
            session,
        } => {
            validate_session(session.as_deref())?;
            validate_ids(connection_id, Some(target_id))?;
        }
        BrowserArgs::Inspect {
            connection_id,
            target_id,
            selector,
            max_chars,
            session,
        } => {
            validate_session(session.as_deref())?;
            validate_ids(connection_id, Some(target_id))?;
            if let Some(selector) = selector {
                bounded(
                    selector,
                    MAX_SELECTOR_BYTES,
                    "El selector excede 4096 bytes",
                )?;
            }
            if max_chars.is_some_and(|value| !(1..=MAX_HTML_CHARS as u32).contains(&value)) {
                return Err(("ARGUMENT", "maxChars debe estar entre 1 y 100000"));
            }
        }
        BrowserArgs::Click {
            connection_id,
            target_id,
            selector,
            session,
        }
        | BrowserArgs::Fill {
            connection_id,
            target_id,
            selector,
            session,
            ..
        }
        | BrowserArgs::Wait {
            connection_id,
            target_id,
            selector,
            session,
            ..
        } => {
            validate_session(session.as_deref())?;
            validate_ids(connection_id, Some(target_id))?;
            if selector.trim().is_empty() {
                return Err(("ARGUMENT", "selector no puede estar vacío"));
            }
            bounded(
                selector,
                MAX_SELECTOR_BYTES,
                "El selector excede 4096 bytes",
            )?;
            if let BrowserArgs::Fill { value, .. } = args {
                bounded(value, MAX_HTML_CHARS, "value excede 100000 bytes")?;
            }
            if let BrowserArgs::Wait {
                timeout_ms: Some(ms),
                ..
            } = args
            {
                if !(1..=15000).contains(ms) {
                    return Err(("ARGUMENT", "timeoutMs debe estar entre 1 y 15000"));
                }
            }
        }
        BrowserArgs::Snapshot {
            connection_id,
            target_id,
            limit,
            session,
        } => {
            validate_session(session.as_deref())?;
            validate_ids(connection_id, Some(target_id))?;
            if limit.is_some_and(|n| !(1..=100).contains(&n)) {
                return Err(("ARGUMENT", "limit debe estar entre 1 y 100"));
            }
        }
        BrowserArgs::Errors {
            connection_id,
            target_id,
            limit,
            session,
            ..
        } => {
            validate_session(session.as_deref())?;
            validate_ids(connection_id, target_id.as_deref())?;
            if limit.is_some_and(|value| !(1..=MAX_EVENTS as u32).contains(&value)) {
                return Err(("ARGUMENT", "limit debe estar entre 1 y 256"));
            }
        }
        BrowserArgs::Screenshot {
            connection_id,
            target_id,
            selector,
            session,
        } => {
            validate_session(session.as_deref())?;
            validate_ids(connection_id, Some(target_id))?;
            if let Some(selector) = selector {
                bounded(
                    selector,
                    MAX_SELECTOR_BYTES,
                    "El selector excede 4096 bytes",
                )?;
            }
        }
    }
    Ok(())
}

/// Accepts direct WebSocket CDP endpoints that resolve textually to loopback only.
pub(crate) fn validate_endpoint(endpoint: &str) -> Result<(), Failure> {
    let url = Url::parse(endpoint)
        .map_err(|_| ("BROWSER_ENDPOINT", "El endpoint CDP no es una URL válida"))?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
    {
        return Err((
            "BROWSER_ENDPOINT",
            "El endpoint CDP no puede incluir credenciales, fragmentos ni query",
        ));
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if !matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1" | "[::1]") {
        return Err((
            "BROWSER_ENDPOINT",
            "Solo se permiten endpoints CDP loopback",
        ));
    }
    match url.scheme() {
        "ws" if url.path().starts_with("/devtools/browser/") => Ok(()),
        _ => Err((
            "BROWSER_ENDPOINT",
            "Use ws://loopback:puerto/devtools/browser/... (WebSocket CDP directo)",
        )),
    }
}

/// Validates a user-supplied connection endpoint without performing network IO.
pub(crate) fn validate_connect_endpoint(endpoint: &str) -> Result<(), Failure> {
    parse_connect_input(endpoint, None).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_ARGUMENT_BYTES, MAX_EVENTS, MAX_HTML_CHARS, parse_connect_input, required_session,
        validate_connect_endpoint, validate_endpoint, validate_request, validate_session_owner,
    };
    use harness_domain::browser::BrowserArgs;

    fn session() -> Option<String> {
        Some("s-1".into())
    }

    #[test]
    fn interaction_request_bounds_reject_invalid_inputs() {
        for value in [
            serde_json::json!({"action":"click","connectionId":"c","targetId":"t","session":"s","selector":""}),
            serde_json::json!({"action":"fill","connectionId":"c","targetId":"t","session":"s","selector":"#x","value":"x".repeat(100001)}),
            serde_json::json!({"action":"wait","connectionId":"c","targetId":"t","session":"s","selector":"#x","timeoutMs":15001}),
            serde_json::json!({"action":"snapshot","connectionId":"c","targetId":"t","session":"s","limit":101}),
            serde_json::json!({"action":"snapshot","connectionId":"c","targetId":"t","session":"s","limit":0}),
            serde_json::json!({"action":"wait","connectionId":"c","targetId":"t","session":"s","selector":"#x","timeoutMs":0}),
        ] {
            let args = serde_json::from_value(value).unwrap();
            assert_eq!(validate_request(&args).unwrap_err().0, "ARGUMENT");
        }
    }

    #[test]
    fn endpoint_policy_accepts_only_local_browser_websockets() {
        assert!(validate_endpoint("ws://localhost:9222/devtools/browser/abc").is_ok());
        assert!(validate_endpoint("ws://127.0.0.1:9222/devtools/browser/abc").is_ok());
        assert!(validate_endpoint("ws://[::1]:9222/devtools/browser/abc").is_ok());
        assert!(validate_endpoint("http://localhost:9222/").is_err());
        assert!(validate_endpoint("http://127.0.0.1:9222/").is_err());
        assert!(validate_endpoint("http://[::1]:9222/").is_err());
        assert!(validate_endpoint("http://example.com:9222/").is_err());
        assert!(validate_endpoint("http://user:pass@localhost:9222/").is_err());
        assert!(validate_endpoint("ws://localhost:9222/devtools/page/abc").is_err());
        assert!(validate_endpoint("http://localhost:9222/?token=secret").is_err());
        assert!(validate_endpoint("ws://localhost:9222/devtools/browser/abc#fragment").is_err());
    }

    #[test]
    fn connect_endpoint_accepts_local_http_discovery_auto_and_rejects_other_http() {
        assert!(validate_connect_endpoint("auto").is_ok());
        assert!(validate_connect_endpoint("localhost").is_ok());
        assert!(validate_connect_endpoint(":5173").is_ok());
        assert!(validate_connect_endpoint("localhost:3000").is_ok());
        assert!(validate_connect_endpoint("http://localhost:9222").is_ok());
        assert!(validate_connect_endpoint("http://127.0.0.1:9222/json/version").is_ok());
        assert!(validate_connect_endpoint("http://[::1]:9222/").is_ok());
        assert!(validate_connect_endpoint("ws://localhost:9222/devtools/browser/id").is_ok());
        for endpoint in [
            "http://example.com:9222/json/version",
            "http://localhost:9222/json/list",
            "http://user:pass@localhost:9222/",
            "http://localhost:9222/?token=x",
            "https://localhost:9222/json/version",
            "http://localhost/json/version",
        ] {
            assert!(validate_connect_endpoint(endpoint).is_err(), "{endpoint}");
        }
    }

    #[test]
    fn parse_connect_input_distinguishes_cdp_endpoints_from_tab_filters() {
        assert_eq!(parse_connect_input("auto", None).unwrap(), (None, None));
        assert_eq!(
            parse_connect_input("localhost:5173", None).unwrap(),
            (None, Some("localhost:5173".into()))
        );
        assert_eq!(
            parse_connect_input(":3000", None).unwrap(),
            (None, Some(":3000".into()))
        );
        assert_eq!(
            parse_connect_input(":9222", Some("my-app")).unwrap(),
            (Some("http://127.0.0.1:9222".into()), Some("my-app".into()))
        );
    }

    #[test]
    fn session_identity_is_required_bounded_and_preserved() {
        assert!(required_session(None).is_err());
        assert!(required_session(Some("  \t".into())).is_err());
        assert_eq!(required_session(Some("s-1".into())).unwrap(), "s-1");
        let spaced = " session ".to_owned();
        assert_eq!(required_session(Some(spaced.clone())).unwrap(), spaced);
        assert!(required_session(Some("x".repeat(MAX_ARGUMENT_BYTES + 1))).is_err());
        assert!(validate_session_owner("s-1", "s-1").is_ok());
        assert_eq!(
            validate_session_owner("s-1", "s-2").unwrap_err().0,
            "BROWSER_SESSION"
        );
    }

    #[test]
    fn request_limits_keep_existing_boundaries() {
        let prefix = "ws://localhost:9222/devtools/browser/";
        let valid = BrowserArgs::Connect {
            endpoint: format!("{prefix}{}", "x".repeat(4_096 - prefix.len())),
            tab: None,
            session: session(),
        };
        assert!(validate_request(&valid).is_ok());
        let overlong_endpoint = BrowserArgs::Connect {
            endpoint: "x".repeat(4_097),
            tab: None,
            session: session(),
        };
        assert_eq!(
            validate_request(&overlong_endpoint).unwrap_err().0,
            "ARGUMENT"
        );

        let valid_ids = BrowserArgs::Attach {
            connection_id: "c".repeat(MAX_ARGUMENT_BYTES),
            target_id: "t".repeat(MAX_ARGUMENT_BYTES),
            session: session(),
        };
        assert!(validate_request(&valid_ids).is_ok());
        let overlong_id = BrowserArgs::Targets {
            connection_id: "c".repeat(MAX_ARGUMENT_BYTES + 1),
            tab: None,
            session: session(),
        };
        assert_eq!(validate_request(&overlong_id).unwrap_err().0, "ARGUMENT");

        let valid_inspect = BrowserArgs::Inspect {
            connection_id: "c".into(),
            target_id: "t".into(),
            selector: Some("x".repeat(4_096)),
            max_chars: Some(MAX_HTML_CHARS as u32),
            session: session(),
        };
        assert!(validate_request(&valid_inspect).is_ok());
        let over_limit_inspect = BrowserArgs::Inspect {
            connection_id: "c".into(),
            target_id: "t".into(),
            selector: Some("x".repeat(4_097)),
            max_chars: Some(MAX_HTML_CHARS as u32 + 1),
            session: session(),
        };
        assert_eq!(
            validate_request(&over_limit_inspect).unwrap_err().0,
            "ARGUMENT"
        );

        let valid_errors = BrowserArgs::Errors {
            connection_id: "c".into(),
            target_id: None,
            after: None,
            limit: Some(MAX_EVENTS as u32),
            session: session(),
        };
        assert!(validate_request(&valid_errors).is_ok());
        let over_limit_errors = BrowserArgs::Errors {
            connection_id: "c".into(),
            target_id: None,
            after: None,
            limit: Some(MAX_EVENTS as u32 + 1),
            session: session(),
        };
        assert_eq!(
            validate_request(&over_limit_errors).unwrap_err().0,
            "ARGUMENT"
        );
    }
}

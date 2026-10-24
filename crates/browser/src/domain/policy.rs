use harness_domain::{Failure, browser::BrowserArgs};
use url::Url;

pub(crate) const MAX_EVENTS: usize = 256;
pub(crate) const MAX_EVENT_MESSAGE: usize = 2_048;
pub(crate) const MAX_HTML_CHARS: usize = 100_000;

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

/// Enforces the adapter's request bounds before any actor or browser work starts.
pub(crate) fn validate_request(args: &BrowserArgs) -> Result<(), Failure> {
    match args {
        BrowserArgs::Connect { endpoint, session } => {
            validate_session(session.as_deref())?;
            bounded(
                endpoint,
                MAX_ENDPOINT_BYTES,
                "El endpoint excede 4096 bytes",
            )?;
        }
        BrowserArgs::Targets {
            connection_id,
            session,
        }
        | BrowserArgs::Disconnect {
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

#[cfg(test)]
mod tests {
    use super::{
        MAX_ARGUMENT_BYTES, MAX_EVENTS, MAX_HTML_CHARS, required_session, validate_endpoint,
        validate_request, validate_session_owner,
    };
    use harness_domain::browser::BrowserArgs;

    fn session() -> Option<String> {
        Some("s-1".into())
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
        let valid = BrowserArgs::Connect {
            endpoint: "x".repeat(4_096),
            session: session(),
        };
        assert!(validate_request(&valid).is_ok());
        let overlong_endpoint = BrowserArgs::Connect {
            endpoint: "x".repeat(4_097),
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

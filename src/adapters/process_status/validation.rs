use harness_domain::{Failure, contracts::ProcessStatusArgs};
use std::time::Duration;

pub(super) struct HttpUrl {
    pub(super) host: String,
    pub(super) port: u16,
    pub(super) path: String,
}

pub(super) struct Options {
    pub(super) health_url: Option<HttpUrl>,
    pub(super) timeout: Duration,
}

pub(super) fn parse_health_url(raw: &str) -> Result<HttpUrl, Failure> {
    if raw.len() > 4096
        || !raw.is_ascii()
        || raw
            .bytes()
            .any(|b| b.is_ascii_control() || b.is_ascii_whitespace() || b == b'\\')
    {
        return Err(("ARGUMENT", "Invalid healthUrl characters"));
    }
    for (index, byte) in raw.bytes().enumerate() {
        if byte == b'%'
            && !raw
                .as_bytes()
                .get(index + 1..index + 3)
                .is_some_and(|pair| pair.iter().all(u8::is_ascii_hexdigit))
        {
            return Err(("ARGUMENT", "Invalid healthUrl percent escape"));
        }
    }
    let (scheme, rest) = raw
        .split_once("://")
        .ok_or(("ARGUMENT", "healthUrl inválida"))?;
    if scheme == "https" {
        return Err((
            "UNSUPPORTED",
            "HTTPS health probes require TLS and are not supported; use HTTP loopback",
        ));
    }
    if scheme != "http" {
        return Err((
            "ARGUMENT",
            "healthUrl requiere HTTP(S) loopback sin credenciales, query ni fragmento",
        ));
    }
    let default_port = 80;
    if rest.contains('@') || rest.contains('?') || rest.contains('#') {
        return Err((
            "ARGUMENT",
            "healthUrl requiere HTTP(S) loopback sin credenciales, query ni fragmento",
        ));
    }
    let (authority, path) = rest.split_once('/').map_or_else(
        || (rest, "/".to_string()),
        |(authority, path)| (authority, format!("/{path}")),
    );

    let (host, port) = if authority.starts_with('[') {
        let (bracketed, port_part) = authority
            .split_once(']')
            .ok_or(("ARGUMENT", "healthUrl inválida"))?;
        let ip = &bracketed[1..];
        let p = if let Some(p_str) = port_part.strip_prefix(':') {
            if p_str.is_empty() || !p_str.bytes().all(|b| b.is_ascii_digit()) {
                return Err(("ARGUMENT", "Invalid healthUrl port"));
            }
            p_str
                .parse::<u16>()
                .map_err(|_| ("ARGUMENT", "Puerto de healthUrl inválido"))?
        } else if port_part.is_empty() {
            default_port
        } else {
            return Err(("ARGUMENT", "Invalid trailing IPv6 authority characters"));
        };
        (ip.to_string(), p)
    } else if let Some((h, p_str)) = authority.split_once(':') {
        if p_str.is_empty() || !p_str.bytes().all(|b| b.is_ascii_digit()) {
            return Err(("ARGUMENT", "Invalid healthUrl port"));
        }
        let p = p_str
            .parse::<u16>()
            .map_err(|_| ("ARGUMENT", "Puerto de healthUrl inválido"))?;
        (h.to_string(), p)
    } else {
        (authority.to_string(), default_port)
    };

    if host != "127.0.0.1" && host != "::1" && host != "localhost" {
        return Err((
            "ARGUMENT",
            "healthUrl requiere HTTP(S) loopback sin credenciales, query ni fragmento",
        ));
    }
    if port == 0 {
        return Err(("ARGUMENT", "Puerto de healthUrl inválido"));
    }

    Ok(HttpUrl { host, port, path })
}

pub(super) fn validate(args: &ProcessStatusArgs) -> Result<Options, Failure> {
    if args.pid.is_none()
        && args.run_id.is_none()
        && args.list.is_none()
        && args.port.is_none()
        && args.health_url.is_none()
    {
        return Err(("ARGUMENT", "Indicar pid, runId, list, port o healthUrl"));
    }
    if let Some(pid) = args.pid
        && (pid == 0 || pid > 2_147_483_647)
    {
        return Err(("ARGUMENT", "pid debe ser un entero positivo"));
    }
    if let Some(run_id) = &args.run_id
        && !harness_domain::execution::valid_request_id(run_id)
    {
        return Err(("ARGUMENT", "runId inválido"));
    }
    if let Some(limit) = args.limit
        && (limit == 0 || limit > 20)
    {
        return Err(("ARGUMENT", "limit: 1–20"));
    }
    if let Some(port) = args.port
        && port == 0
    {
        return Err(("ARGUMENT", "port: 1–65535"));
    }
    if let Some(host) = &args.host
        && host != "127.0.0.1"
        && host != "::1"
    {
        return Err(("ARGUMENT", "host solo admite loopback numérico"));
    }
    let timeout_ms = args.timeout_ms.unwrap_or(1000);
    if !(50..=5000).contains(&timeout_ms) {
        return Err(("ARGUMENT", "timeoutMs: 50–5000"));
    }
    let timeout = Duration::from_millis(timeout_ms);

    let health_url = args
        .health_url
        .as_deref()
        .map(parse_health_url)
        .transpose()?;

    Ok(Options {
        health_url,
        timeout,
    })
}

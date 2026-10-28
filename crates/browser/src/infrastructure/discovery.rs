use std::{
    collections::HashSet,
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};

use harness_domain::{Failure, browser::BrowserTarget};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, lookup_host},
};
use url::{Position, Url};

use crate::{
    domain::{
        MAX_EVENT_MESSAGE, is_auto_endpoint, is_localhost_url, parse_connect_input, rank_targets,
        validate_connect_endpoint, validate_endpoint,
    },
    infrastructure::{cdp::MAX_TARGETS, events},
};

pub(crate) const STANDARD_CDP_PORTS: [u16; 4] = [9222, 9229, 9223, 9224];

const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const AUTO_PROBE_TIMEOUT: Duration = Duration::from_millis(300);
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_RESOLVED_ADDRESSES: usize = 8;

#[derive(Debug, Clone)]
pub(crate) struct DiscoveredSession {
    pub(crate) ws_endpoint: String,
    #[allow(dead_code)]
    pub(crate) http_endpoint: Option<String>,
    pub(crate) targets: Vec<BrowserTarget>,
    pub(crate) effective_tab_filter: Option<String>,
}

fn invalid_endpoint() -> Failure {
    ("BROWSER_ENDPOINT", "Use una URL HTTP local de descubrimiento CDP")
}

fn connect_failure() -> Failure {
    ("BROWSER_CONNECT", "No se pudo descubrir el endpoint CDP local")
}

pub(super) fn validate_discovery_url(endpoint: &str) -> Result<Url, Failure> {
    let url = Url::parse(endpoint)
        .map_err(|_| ("BROWSER_ENDPOINT", "El endpoint CDP no es una URL válida"))?;
    validate_connect_endpoint(endpoint)?;
    if url.scheme() != "http" {
        return Err(invalid_endpoint());
    }
    Ok(url)
}

fn response_body(response: &[u8]) -> Result<&[u8], Failure> {
    if response.len() > MAX_RESPONSE_BYTES {
        return Err(connect_failure());
    }
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(connect_failure)?;
    let headers = std::str::from_utf8(&response[..header_end]).map_err(|_| connect_failure())?;
    let mut lines = headers.split("\r\n");
    let status = lines.next().ok_or_else(connect_failure)?;
    if !status.starts_with("HTTP/1.0 200 ") && !status.starts_with("HTTP/1.1 200 ") {
        // Redirects and every non-success status are rejected; no Location is followed.
        return Err(connect_failure());
    }
    let mut content_length = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(connect_failure());
        };
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(connect_failure());
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(connect_failure());
            }
            content_length = Some(value.trim().parse::<usize>().map_err(|_| connect_failure())?);
        }
    }
    let body = &response[header_end + 4..];
    let expected = content_length.ok_or_else(connect_failure)?;
    if expected > MAX_RESPONSE_BYTES || body.len() != expected {
        return Err(connect_failure());
    }
    Ok(body)
}

fn pin_websocket_to_peer(
    endpoint: &str,
    discovery_port: u16,
    peer: SocketAddr,
) -> Result<String, Failure> {
    validate_endpoint(endpoint)?;
    let mut url = Url::parse(endpoint).map_err(|_| connect_failure())?;
    if url.port() != Some(discovery_port) {
        return Err(connect_failure());
    }

    // Pin the WebSocket connection to the loopback address used for discovery, so
    // Chromiumoxide does not resolve a hostname again after this security check.
    let pinned_host = match peer.ip() {
        std::net::IpAddr::V4(address) => address.to_string(),
        std::net::IpAddr::V6(address) => format!("[{address}]"),
    };
    url.set_host(Some(&pinned_host)).map_err(|_| connect_failure())?;
    validate_endpoint(url.as_str())?;
    Ok(url.into())
}

fn websocket_endpoint(body: &[u8], discovery_port: u16, peer: SocketAddr) -> Result<String, Failure> {
    let json: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| connect_failure())?;
    let endpoint = json
        .get("webSocketDebuggerUrl")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(connect_failure)?;
    pin_websocket_to_peer(endpoint, discovery_port, peer)
}

/// Parses a `/json/list` or `/json` CDP HTTP response into ranked `BrowserTarget` items.
pub(crate) fn parse_target_list_json(
    body: &[u8],
    filter: Option<&str>,
) -> Result<Vec<BrowserTarget>, Failure> {
    let items: Vec<serde_json::Value> =
        serde_json::from_slice(body).map_err(|_| connect_failure())?;
    let mut targets = Vec::new();
    for item in items {
        let target_type = item
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("page");
        if target_type != "page" {
            continue;
        }
        if targets.len() >= MAX_TARGETS {
            break;
        }
        let Some(id) = item
            .get("id")
            .or_else(|| item.get("targetId"))
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        let title = item
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let url = item
            .get("url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let is_localhost = is_localhost_url(&url);
        targets.push(BrowserTarget::new(
            id,
            events::bounded_text(title, MAX_EVENT_MESSAGE),
            events::bounded_text(url, MAX_EVENT_MESSAGE),
            target_type,
            is_localhost,
        ));
    }
    rank_targets(&mut targets, filter);
    Ok(targets)
}

/// Parses a `DevToolsActivePort` file content (`"<port>\n<ws_path>"`) into `(port, ws_path)`.
pub(crate) fn parse_devtools_active_port(content: &str) -> Option<(u16, String)> {
    let mut lines = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let port: u16 = lines.next()?.parse().ok()?;
    if port == 0 {
        return None;
    }
    let path = lines
        .next()
        .filter(|path| path.starts_with("/devtools/browser/"))
        .unwrap_or("/devtools/browser/")
        .to_owned();
    Some((port, path))
}

/// Returns candidate `DevToolsActivePort` file paths across standard Chrome, Edge, Brave,
/// and Chromium user-data directories on Windows, macOS, and Linux.
pub(crate) fn default_devtools_active_port_files() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let base = PathBuf::from(local_app_data);
        for rel in [
            "Google/Chrome/User Data",
            "Google/Chrome Beta/User Data",
            "Google/Chrome SxS/User Data",
            "Microsoft/Edge/User Data",
            "BraveSoftware/Brave-Browser/User Data",
            "Chromium/User Data",
        ] {
            roots.push(base.join(rel));
        }
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let home = PathBuf::from(home);
        for rel in [
            // macOS
            "Library/Application Support/Google/Chrome",
            "Library/Application Support/Google/Chrome Canary",
            "Library/Application Support/Microsoft Edge",
            "Library/Application Support/BraveSoftware/Brave-Browser",
            "Library/Application Support/Chromium",
            // Linux
            ".config/google-chrome",
            ".config/google-chrome-beta",
            ".config/microsoft-edge",
            ".config/BraveSoftware/Brave-Browser",
            ".config/chromium",
        ] {
            roots.push(home.join(rel));
        }
    }
    roots
        .into_iter()
        .map(|root| root.join("DevToolsActivePort"))
        .collect()
}

/// Reads candidate `(port, Option<ws_endpoint>)` entries from `DevToolsActivePort` files
/// and standard local CDP ports (`9222`, `9229`, `9223`, `9224`).
pub(crate) fn collect_auto_discovery_candidates(
    active_port_files: &[PathBuf],
    standard_ports: &[u16],
) -> Vec<(u16, Option<String>)> {
    let mut seen_ports = HashSet::new();
    let mut candidates = Vec::new();

    for path in active_port_files {
        if let Some((port, ws_path)) = read_devtools_active_port_file(path) {
            if seen_ports.insert(port) {
                let ws_url = if ws_path.len() > "/devtools/browser/".len() {
                    Some(format!("ws://127.0.0.1:{port}{ws_path}"))
                } else {
                    None
                };
                candidates.push((port, ws_url));
            }
        }
    }

    for &port in standard_ports {
        if seen_ports.insert(port) {
            candidates.push((port, None));
        }
    }

    candidates
}

fn read_devtools_active_port_file(path: &Path) -> Option<(u16, String)> {
    let content = std::fs::read_to_string(path).ok()?;
    parse_devtools_active_port(&content)
}

#[allow(dead_code)]
pub(super) async fn resolve_endpoint(endpoint: &str) -> Result<String, Failure> {
    if is_auto_endpoint(endpoint) {
        let session = discover_session(Some(endpoint), None).await?;
        return Ok(session.ws_endpoint);
    }
    if endpoint
        .get(..5)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("ws://"))
    {
        validate_endpoint(endpoint)?;
        return Ok(endpoint.to_owned());
    }

    let url = validate_discovery_url(endpoint)?;
    tokio::time::timeout(DISCOVERY_TIMEOUT, resolve_discovery(url))
        .await
        .map_err(|_| connect_failure())?
}

/// Discovers a local CDP session and its open `page` targets (via `/json/list` when available).
pub(crate) async fn discover_session(
    endpoint: Option<&str>,
    tab_filter: Option<&str>,
) -> Result<DiscoveredSession, Failure> {
    let raw_endpoint = endpoint.unwrap_or("auto");
    let (resolved_cdp, effective_tab_filter) = parse_connect_input(raw_endpoint, tab_filter)?;

    match resolved_cdp {
        Some(explicit) if explicit
            .get(..5)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("ws://")) =>
        {
            validate_endpoint(&explicit)?;
            let url = Url::parse(&explicit).map_err(|_| invalid_endpoint())?;
            let targets = if let Some(port) = url.port() {
                let host = url.host_str().unwrap_or("127.0.0.1");
                let http_url = Url::parse(&format!("http://{host}:{port}")).ok();
                if let Some(http_url) = http_url {
                    fetch_targets_http(&http_url, AUTO_PROBE_TIMEOUT, effective_tab_filter.as_deref())
                        .await
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            Ok(DiscoveredSession {
                ws_endpoint: explicit,
                http_endpoint: None,
                targets,
                effective_tab_filter,
            })
        }
        Some(explicit) => {
            let url = validate_discovery_url(&explicit)?;
            let ws_endpoint = tokio::time::timeout(DISCOVERY_TIMEOUT, resolve_discovery(url.clone()))
                .await
                .map_err(|_| connect_failure())??;
            let targets = fetch_targets_http(&url, AUTO_PROBE_TIMEOUT, effective_tab_filter.as_deref())
                .await
                .unwrap_or_default();
            let authority = &url[Position::BeforeHost..Position::AfterPort];
            Ok(DiscoveredSession {
                ws_endpoint,
                http_endpoint: Some(format!("http://{authority}")),
                targets,
                effective_tab_filter,
            })
        }
        None => {
            let active_files = default_devtools_active_port_files();
            let candidates =
                collect_auto_discovery_candidates(&active_files, &STANDARD_CDP_PORTS);
            discover_from_candidates(&candidates, effective_tab_filter).await
        }
    }
}

pub(crate) async fn discover_from_candidates(
    candidates: &[(u16, Option<String>)],
    effective_tab_filter: Option<String>,
) -> Result<DiscoveredSession, Failure> {
    let mut best_fallback: Option<DiscoveredSession> = None;

    for (port, active_ws_hint) in candidates {
        let Ok(url) = Url::parse(&format!("http://127.0.0.1:{port}")) else {
            continue;
        };
        let ws_from_http = tokio::time::timeout(AUTO_PROBE_TIMEOUT, resolve_discovery(url.clone()))
            .await
            .ok()
            .and_then(Result::ok);

        let ws_endpoint = match (ws_from_http, active_ws_hint) {
            (Some(ws), _) => ws,
            (None, Some(hint)) if validate_endpoint(hint).is_ok() => {
                // Verify TCP port is actually reachable before accepting a stale DevToolsActivePort file
                let addr = SocketAddr::from(([127, 0, 0, 1], *port));
                if tokio::time::timeout(AUTO_PROBE_TIMEOUT, TcpStream::connect(addr))
                    .await
                    .is_ok_and(|res| res.is_ok())
                {
                    hint.clone()
                } else {
                    continue;
                }
            }
            _ => continue,
        };

        let targets = fetch_targets_http(&url, AUTO_PROBE_TIMEOUT, effective_tab_filter.as_deref())
            .await
            .unwrap_or_default();
        let session = DiscoveredSession {
            ws_endpoint,
            http_endpoint: Some(format!("http://127.0.0.1:{port}")),
            targets,
            effective_tab_filter: effective_tab_filter.clone(),
        };

        // If this candidate has a matching tab (or a localhost tab when no filter), prefer it immediately!
        if crate::domain::select_target(&session.targets, effective_tab_filter.as_deref()).is_some()
        {
            return Ok(session);
        }
        if best_fallback.is_none() {
            best_fallback = Some(session);
        }
    }

    best_fallback.ok_or_else(connect_failure)
}

async fn resolve_discovery(url: Url) -> Result<String, Failure> {
    let (peer, port, body) = http_get(&url, "/json/version", CONNECT_TIMEOUT).await?;
    websocket_endpoint(&body, port, peer)
}

async fn fetch_targets_http(
    url: &Url,
    connect_timeout: Duration,
    filter: Option<&str>,
) -> Result<Vec<BrowserTarget>, Failure> {
    let (_, _, body) = http_get(url, "/json/list", connect_timeout).await?;
    parse_target_list_json(&body, filter)
}

async fn http_get(
    url: &Url,
    path: &str,
    connect_timeout: Duration,
) -> Result<(SocketAddr, u16, Vec<u8>), Failure> {
    let host = url.host_str().ok_or_else(invalid_endpoint)?;
    let host_for_lookup = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    let port = url.port().ok_or_else(invalid_endpoint)?;
    let addresses = lookup_host((host_for_lookup, port))
        .await
        .map_err(|_| connect_failure())?
        .collect::<Vec<_>>();
    if addresses.is_empty()
        || addresses.len() > MAX_RESOLVED_ADDRESSES
        || addresses.iter().any(|address| !address.ip().is_loopback())
    {
        return Err(invalid_endpoint());
    }

    let mut stream = None;
    for address in addresses {
        if let Ok(Ok(connected)) =
            tokio::time::timeout(connect_timeout, TcpStream::connect(address)).await
        {
            stream = Some(connected);
            break;
        }
    }
    let mut stream = stream.ok_or_else(connect_failure)?;
    let peer = stream.peer_addr().map_err(|_| connect_failure())?;
    if !peer.ip().is_loopback() {
        return Err(invalid_endpoint());
    }

    let authority = &url[Position::BeforeHost..Position::AfterPort];
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {authority}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    let mut response = Vec::new();
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|_| connect_failure())?;
    stream
        .take((MAX_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut response)
        .await
        .map_err(|_| connect_failure())?;

    let body = response_body(&response)?.to_vec();
    Ok((peer, port, body))
}

#[cfg(test)]
mod tests {
    use super::{
        collect_auto_discovery_candidates, discover_from_candidates, parse_devtools_active_port,
        parse_target_list_json, resolve_endpoint, response_body, validate_discovery_url,
        websocket_endpoint,
    };
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[test]
    fn discovery_policy_accepts_only_local_http_version_endpoints() {
        for endpoint in [
            "http://localhost:9222",
            "http://127.0.0.1:9222/",
            "http://[::1]:9222/json/version",
        ] {
            assert!(validate_discovery_url(endpoint).is_ok(), "{endpoint}");
        }
        for endpoint in [
            "https://localhost:9222/json/version",
            "http://example.com:9222/json/version",
            "http://user:pass@localhost:9222/json/version",
            "http://localhost:9222/json/version?token=x",
            "http://localhost:9222/json/version#fragment",
            "http://localhost:9222/json/list",
            "http://localhost/json/version",
        ] {
            assert!(validate_discovery_url(endpoint).is_err(), "{endpoint}");
        }
    }

    #[test]
    fn discovery_response_requires_bounded_success_with_content_length() {
        let body = br#"{"webSocketDebuggerUrl":"ws://localhost:9222/devtools/browser/id"}"#;
        let ok = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            std::str::from_utf8(body).unwrap()
        );
        assert_eq!(response_body(ok.as_bytes()).unwrap(), body);
        assert!(response_body(b"HTTP/1.1 302 Found\r\nLocation: http://example.com\r\n\r\n").is_err());
        assert!(response_body(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n").is_err());
    }

    #[test]
    fn websocket_discovery_stays_on_loopback_and_discovery_port() {
        let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9222);
        let valid = br#"{"webSocketDebuggerUrl":"ws://localhost:9222/devtools/browser/id"}"#;
        assert_eq!(
            websocket_endpoint(valid, 9222, peer).unwrap(),
            "ws://127.0.0.1:9222/devtools/browser/id"
        );
        let remote = br#"{"webSocketDebuggerUrl":"ws://example.com:9222/devtools/browser/id"}"#;
        let other_port = br#"{"webSocketDebuggerUrl":"ws://localhost:9333/devtools/browser/id"}"#;
        assert!(websocket_endpoint(remote, 9222, peer).is_err());
        assert!(websocket_endpoint(other_port, 9222, peer).is_err());
        assert!(websocket_endpoint(b"{}", 9222, peer).is_err());

        let ipv6_peer = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 9222);
        assert_eq!(
            websocket_endpoint(valid, 9222, ipv6_peer).unwrap(),
            "ws://[::1]:9222/devtools/browser/id"
        );
    }

    #[test]
    fn parses_devtools_active_port_file_and_deduplicates_candidates() {
        let parsed =
            parse_devtools_active_port("9222\n/devtools/browser/3b2a1c-uuid\n").unwrap();
        assert_eq!(parsed.0, 9222);
        assert_eq!(parsed.1, "/devtools/browser/3b2a1c-uuid");

        assert!(parse_devtools_active_port("0\n/devtools/browser/id").is_none());
        assert!(parse_devtools_active_port("not-a-port").is_none());

        let candidates =
            collect_auto_discovery_candidates(&[], &[9222, 9229, 9222, 9223]);
        let ports: Vec<u16> = candidates.into_iter().map(|(port, _)| port).collect();
        assert_eq!(ports, vec![9222, 9229, 9223]);
    }

    #[test]
    fn parses_json_list_and_ranks_localhost_pages_first() {
        let raw = br#"[
            {"id":"sw-1","type":"service_worker","title":"SW","url":"http://localhost:3000/sw.js"},
            {"id":"page-ext","type":"page","title":"GitHub","url":"https://github.com/org/repo"},
            {"id":"page-local","type":"page","title":"Web App","url":"http://localhost:5173/dashboard"}
        ]"#;
        let targets = parse_target_list_json(raw, None).unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].id, "page-local");
        assert!(targets[0].is_localhost);
        assert_eq!(targets[1].id, "page-ext");
        assert!(!targets[1].is_localhost);
    }

    #[tokio::test]
    async fn direct_websocket_scheme_is_case_insensitive_without_discovery_io() {
        let endpoint = "WS://localhost:9222/devtools/browser/id";
        assert_eq!(resolve_endpoint(endpoint).await.unwrap(), endpoint);
    }

    #[tokio::test]
    async fn local_http_discovery_fetches_only_the_version_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let count = stream.read(&mut request).await.unwrap();
            let request = std::str::from_utf8(&request[..count]).unwrap();
            assert!(request.starts_with("GET /json/version HTTP/1.1\r\n"));
            let body = format!(
                r#"{{"webSocketDebuggerUrl":"ws://localhost:{}/devtools/browser/id"}}"#,
                address.port()
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let input = format!("http://127.0.0.1:{}/", address.port());
        let endpoint = resolve_endpoint(&input).await.unwrap();
        assert_eq!(
            endpoint,
            format!("ws://127.0.0.1:{}/devtools/browser/id", address.port())
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn auto_discovery_probes_candidates_and_enumerates_localhost_tabs() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0u8; 1024];
                let count = stream.read(&mut request).await.unwrap();
                let request_str = std::str::from_utf8(&request[..count]).unwrap();
                let body = if request_str.starts_with("GET /json/version HTTP/1.1\r\n") {
                    format!(
                        r#"{{"webSocketDebuggerUrl":"ws://localhost:{port}/devtools/browser/auto-id"}}"#
                    )
                } else if request_str.starts_with("GET /json/list HTTP/1.1\r\n") {
                    r#"[{"id":"tab-docs","type":"page","title":"Docs","url":"https://example.com"},{"id":"tab-app","type":"page","title":"Fast Local","url":"http://localhost:5173/home"}]"#.to_owned()
                } else {
                    panic!("unexpected request: {request_str}");
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });

        let candidates = vec![(9, None), (port, None)];
        let discovered = discover_from_candidates(&candidates, Some(":5173".into()))
            .await
            .unwrap();
        assert_eq!(
            discovered.ws_endpoint,
            format!("ws://127.0.0.1:{port}/devtools/browser/auto-id")
        );
        assert_eq!(
            discovered.http_endpoint.as_deref(),
            Some(format!("http://127.0.0.1:{port}").as_str())
        );
        assert_eq!(discovered.targets.len(), 2);
        assert_eq!(discovered.targets[0].id, "tab-app");
        assert!(discovered.targets[0].is_localhost);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn ipv6_loopback_discovery_pins_the_websocket_to_ipv6() {
        let listener = match TcpListener::bind("[::1]:0").await {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("Skipping IPv6 listener test: loopback IPv6 unavailable: {error}");
                return;
            }
        };
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let count = stream.read(&mut request).await.unwrap();
            let request = std::str::from_utf8(&request[..count]).unwrap();
            assert!(request.starts_with("GET /json/version HTTP/1.1\r\n"));
            let body = format!(
                r#"{{"webSocketDebuggerUrl":"ws://[::1]:{}/devtools/browser/id"}}"#,
                address.port()
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let input = format!("http://[::1]:{}/json/version", address.port());
        let endpoint = resolve_endpoint(&input).await.unwrap();
        assert_eq!(
            endpoint,
            format!("ws://[::1]:{}/devtools/browser/id", address.port())
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn local_http_discovery_rejects_redirects() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: http://example.com/json/version\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });

        let input = format!("http://127.0.0.1:{}/json/version", address.port());
        assert!(resolve_endpoint(&input).await.is_err());
        server.await.unwrap();
    }
}

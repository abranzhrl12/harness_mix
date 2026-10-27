use std::{net::SocketAddr, time::Duration};

use harness_domain::Failure;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, lookup_host},
};
use url::{Position, Url};

use crate::domain::{validate_connect_endpoint, validate_endpoint};

const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_RESOLVED_ADDRESSES: usize = 8;

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

fn websocket_endpoint(body: &[u8], discovery_port: u16, peer: SocketAddr) -> Result<String, Failure> {
    let json: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| connect_failure())?;
    let endpoint = json
        .get("webSocketDebuggerUrl")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(connect_failure)?;
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

pub(super) async fn resolve_endpoint(endpoint: &str) -> Result<String, Failure> {
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

async fn resolve_discovery(url: Url) -> Result<String, Failure> {
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
            tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(address)).await
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
        "GET /json/version HTTP/1.1\r\nHost: {authority}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    let mut response = Vec::new();
    stream.write_all(request.as_bytes()).await.map_err(|_| connect_failure())?;
    stream
        .take((MAX_RESPONSE_BYTES + 1) as u64)
        .read_to_end(&mut response)
        .await
        .map_err(|_| connect_failure())?;

    let body = response_body(&response)?;
    websocket_endpoint(body, port, peer)
}

#[cfg(test)]
mod tests {
    use super::{resolve_endpoint, response_body, validate_discovery_url, websocket_endpoint};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
    use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpListener};

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

//! Native process status, TCP probing and HTTP loopback health checking adapter.
//! Safe, bounded diagnostic tool; no shell, no global termination.

use harness_domain::{Failure, contracts::ProcessStatusArgs, execution::OperationContext};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::{Duration, Instant},
};

struct HttpUrl {
    host: String,
    port: u16,
    path: String,
}

fn parse_health_url(raw: &str) -> Result<HttpUrl, Failure> {
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

fn validate(args: &ProcessStatusArgs) -> Result<(Option<HttpUrl>, Duration), Failure> {
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

    let mut parsed_url = None;
    if let Some(raw_url) = &args.health_url {
        parsed_url = Some(parse_health_url(raw_url)?);
    }

    Ok((parsed_url, timeout))
}

fn check_pid(pid: u32) -> Value {
    #[cfg(windows)]
    {
        use windows::Win32::{
            Foundation::{CloseHandle, STILL_ACTIVE},
            System::Threading::{
                GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        // SAFETY: OpenProcess called with limited query rights on numeric PID.
        unsafe {
            match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                Ok(handle) => {
                    let mut code = 0u32;
                    let queried = GetExitCodeProcess(handle, &mut code);
                    let _ = CloseHandle(handle);
                    if queried.is_err() {
                        json!({"pid":pid,"state":"unknown","reason":"query_failed","responsive":null})
                    } else if code == STILL_ACTIVE.0 as u32 {
                        json!({"pid": pid, "state": "exists", "responsive": null})
                    } else {
                        json!({"pid": pid, "state": "not_running", "responsive": null})
                    }
                }
                Err(error) => {
                    let (state, reason) = windows_open_error(error.code().0 as u32 & 0xffff);
                    json!({"pid":pid,"state":state,"reason":reason,"responsive":null})
                }
            }
        }
    }
    #[cfg(unix)]
    {
        let res = unsafe { libc::kill(pid as i32, 0) };
        if res == 0 {
            json!({"pid": pid, "state": "exists", "responsive": null})
        } else {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ESRCH) {
                json!({"pid": pid, "state": "not_running", "responsive": null})
            } else if err.raw_os_error() == Some(libc::EPERM) {
                json!({"pid": pid, "state": "unknown", "reason": "access_denied", "responsive": null})
            } else {
                json!({"pid": pid, "state": "unknown", "reason": "probe_failed", "responsive": null})
            }
        }
    }
}

#[cfg(any(windows, test))]
fn windows_open_error(code: u32) -> (&'static str, &'static str) {
    match code {
        87 | 1168 => ("not_running", "not_found"),
        5 => ("unknown", "access_denied"),
        _ => ("unknown", "probe_failed"),
    }
}

fn probe_tcp(host: &str, port: u16, timeout: Duration) -> Value {
    let started = Instant::now();
    let addr_str = if host == "::1" {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };

    let addrs: Vec<SocketAddr> = match addr_str.parse() {
        Ok(a) => vec![a],
        Err(_) => {
            return json!({
                "host": host,
                "port": port,
                "state": "unknown",
                "accepting": null,
                "milliseconds": started.elapsed().as_millis() as u64
            });
        }
    };

    let mut stream_res = Err(std::io::Error::other("no address"));
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(s) => {
                stream_res = Ok(s);
                break;
            }
            Err(e) => {
                stream_res = Err(e);
            }
        }
    }

    let elapsed = started.elapsed().as_millis() as u64;
    match stream_res {
        Ok(_) => json!({
            "host": host,
            "port": port,
            "state": "accepting",
            "accepting": true,
            "milliseconds": elapsed
        }),
        Err(err) if err.kind() == std::io::ErrorKind::TimedOut => json!({
            "host": host,
            "port": port,
            "state": "timeout",
            "accepting": null,
            "milliseconds": elapsed
        }),
        Err(err)
            if err.kind() == std::io::ErrorKind::ConnectionRefused
                || err.kind() == std::io::ErrorKind::ConnectionReset =>
        {
            json!({
                "host": host,
                "port": port,
                "state": "refused",
                "accepting": false,
                "milliseconds": elapsed
            })
        }
        Err(_) => json!({
            "host": host,
            "port": port,
            "state": "unknown",
            "accepting": null,
            "milliseconds": elapsed
        }),
    }
}

fn probe_http(url: &HttpUrl, timeout: Duration) -> Value {
    let started = Instant::now();
    let host = &url.host;
    let port = url.port;
    let resolved_host = if host == "localhost" {
        "127.0.0.1"
    } else {
        host
    };
    let addr_str = if resolved_host == "::1" {
        format!("[{resolved_host}]:{port}")
    } else {
        format!("{resolved_host}:{port}")
    };

    let stream = match TcpStream::connect_timeout(
        &match addr_str.parse::<SocketAddr>() {
            Ok(a) => a,
            Err(_) => {
                return json!({
                    "state": "error",
                    "responding": null,
                    "healthy": null,
                    "reason": "request_failed",
                    "milliseconds": started.elapsed().as_millis() as u64
                });
            }
        },
        timeout,
    ) {
        Ok(s) => s,
        Err(err) if err.kind() == std::io::ErrorKind::TimedOut => {
            return json!({
                "state": "timeout",
                "responding": null,
                "healthy": null,
                "milliseconds": started.elapsed().as_millis() as u64
            });
        }
        Err(err) if err.kind() == std::io::ErrorKind::ConnectionRefused => {
            return json!({
                "state": "refused",
                "responding": false,
                "healthy": null,
                "reason": "connection_refused",
                "milliseconds": started.elapsed().as_millis() as u64
            });
        }
        Err(_) => {
            return json!({
                "state": "error",
                "responding": null,
                "healthy": null,
                "reason": "request_failed",
                "milliseconds": started.elapsed().as_millis() as u64
            });
        }
    };

    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    let path = if url.path.is_empty() {
        "/"
    } else {
        url.path.as_str()
    };
    let authority = if host == "::1" {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let request_data = format!(
        "GET {path} HTTP/1.1\r\nHost: {authority}\r\nUser-Agent: agent-harness\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );

    let mut stream = stream;
    if stream.write_all(request_data.as_bytes()).is_err() {
        return json!({
            "state": "error",
            "responding": null,
            "healthy": null,
            "reason": "request_failed",
            "milliseconds": started.elapsed().as_millis() as u64
        });
    }

    let mut buffer = [0u8; 1024];
    match stream.read(&mut buffer) {
        Ok(n) if n > 0 => {
            let response_text = String::from_utf8_lossy(&buffer[..n]);
            let status_code = response_text
                .lines()
                .next()
                .and_then(|line| {
                    let mut parts = line.split_whitespace();
                    let _http = parts.next()?;
                    parts.next()?.parse::<u16>().ok()
                })
                .unwrap_or(0);

            let elapsed = started.elapsed().as_millis() as u64;
            let healthy = (200..300).contains(&status_code);
            json!({
                "state": "responded",
                "responding": true,
                "healthy": healthy,
                "statusCode": status_code,
                "milliseconds": elapsed
            })
        }
        Ok(_) => json!({
            "state": "error",
            "responding": false,
            "healthy": null,
            "reason": "request_failed",
            "milliseconds": started.elapsed().as_millis() as u64
        }),
        Err(err) if err.kind() == std::io::ErrorKind::TimedOut => json!({
            "state": "timeout",
            "responding": null,
            "healthy": null,
            "milliseconds": started.elapsed().as_millis() as u64
        }),
        Err(_) => json!({
            "state": "error",
            "responding": null,
            "healthy": null,
            "reason": "request_failed",
            "milliseconds": started.elapsed().as_millis() as u64
        }),
    }
}

pub fn handle_process_status(
    args: &ProcessStatusArgs,
    context: &OperationContext,
) -> Result<Value, Failure> {
    handle_process_status_with_tasks(args, context, None)
}
pub fn handle_process_status_with_tasks(
    args: &ProcessStatusArgs,
    context: &OperationContext,
    tasks: Option<&super::tasks::TaskManager>,
) -> Result<Value, Failure> {
    context.check()?;
    let (url, timeout) = validate(args)?;
    let started = Instant::now();

    let mut result = json!({
        "sampledAt": chrono::Utc::now().to_rfc3339(),
    });

    if let Some(pid) = args.pid {
        result["process"] = check_pid(pid);
    }

    if args.run_id.is_some() || args.list.is_some() {
        let jobs = tasks
            .ok_or(("UNSUPPORTED", "Managed job registry is not attached"))?
            .process_status(args)?;
        for (key, value) in jobs
            .as_object()
            .ok_or(("INTERNAL", "Invalid job status result"))?
        {
            result[key] = value.clone();
        }
    }

    if let Some(port) = args.port {
        let host = args.host.as_deref().unwrap_or("127.0.0.1");
        context.check()?;
        result["tcp"] = probe_tcp(host, port, timeout);
    }

    if let Some(u) = &url {
        context.check()?;
        result["health"] = probe_http(u, timeout);
    }

    result["milliseconds"] = json!(started.elapsed().as_millis() as u64);
    result["limitations"] = json!(
        "Point-in-time observations. PID existence does not prove responsiveness or identity (PID reuse). TCP acceptance is not application health or ownership by the requested PID. HTTP health means a 2xx status only, not body validation. Run IDs refer to retained harness_jobs task IDs; history survives restart and interrupted work is not rerun."
    );

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn health_urls_reject_tls_and_request_injection() {
        assert_eq!(
            parse_health_url("https://127.0.0.1:1/health")
                .err()
                .unwrap()
                .0,
            "UNSUPPORTED"
        );
        for url in [
            "http://127.0.0.1/\r\nInjected:yes",
            "http://127.0.0.1/has space",
            "http://[::1]garbage/",
            "http://[::1]:+80/",
            "http://[::1]:80:90/",
            "http://127.0.0.1:80junk/",
            "http://127.0.0.1\\evil/",
            "http://localhost@evil/",
            "http://127.0.0.1/?x=1",
        ] {
            assert!(parse_health_url(url).is_err(), "{url:?}");
        }
        let parsed = parse_health_url("http://[::1]:1234/health").unwrap();
        assert_eq!(parsed.host, "::1");
        assert_eq!(parsed.port, 1234);
        assert_eq!(parsed.path, "/health");
    }
    #[test]
    fn windows_probe_errors_do_not_claim_process_absence() {
        assert_eq!(windows_open_error(5), ("unknown", "access_denied"));
        assert_eq!(windows_open_error(87), ("not_running", "not_found"));
        assert_eq!(windows_open_error(1168), ("not_running", "not_found"));
        assert_eq!(windows_open_error(12345), ("unknown", "probe_failed"));
    }
    #[test]
    fn ipv6_loopback_http_uses_bracketed_socket_and_host_header() {
        let Ok(listener) = std::net::TcpListener::bind("[::1]:0") else {
            return;
        };
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let started = Instant::now();
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut bytes = [0; 1024];
                        let size = stream.read(&mut bytes).unwrap();
                        assert!(
                            String::from_utf8_lossy(&bytes[..size])
                                .contains(&format!("Host: [::1]:{port}\r\n"))
                        );
                        stream
                            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                            .unwrap();
                        break;
                    }
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && started.elapsed() < Duration::from_secs(2) =>
                    {
                        std::thread::sleep(Duration::from_millis(2))
                    }
                    Err(error) => panic!("IPv6 HTTP probe failed: {error}"),
                }
            }
        });
        let url = parse_health_url(&format!("http://[::1]:{port}/health")).unwrap();
        assert_eq!(probe_http(&url, Duration::from_secs(1))["healthy"], true);
        server.join().unwrap();
    }
}

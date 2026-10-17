//! Native process status, TCP probing and HTTP loopback health checking adapter.
//! Safe, bounded diagnostic tool; no shell, no global termination.

use harness_domain::{
    Failure,
    contracts::ProcessStatusArgs,
    execution::OperationContext,
};
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
    let (scheme, rest) = raw
        .split_once("://")
        .ok_or(("ARGUMENT", "healthUrl inválida"))?;
    if scheme != "http" && scheme != "https" {
        return Err((
            "ARGUMENT",
            "healthUrl requiere HTTP(S) loopback sin credenciales, query ni fragmento",
        ));
    }
    let default_port = if scheme == "https" { 443 } else { 80 };
    if rest.contains('@') || rest.contains('?') || rest.contains('#') {
        return Err((
            "ARGUMENT",
            "healthUrl requiere HTTP(S) loopback sin credenciales, query ni fragmento",
        ));
    }
    let (authority, path_part) = rest.split_once('/').map_or((rest, "/"), |(a, p)| (a, p));
    let path = if path_part.starts_with('/') {
        path_part.to_string()
    } else {
        format!("/{path_part}")
    };

    let (host, port) = if authority.starts_with('[') {
        let (bracketed, port_part) = authority
            .split_once(']')
            .ok_or(("ARGUMENT", "healthUrl inválida"))?;
        let ip = &bracketed[1..];
        let p = if let Some(p_str) = port_part.strip_prefix(':') {
            p_str
                .parse::<u16>()
                .map_err(|_| ("ARGUMENT", "Puerto de healthUrl inválido"))?
        } else {
            default_port
        };
        (ip.to_string(), p)
    } else if let Some((h, p_str)) = authority.split_once(':') {
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
        && (run_id.len() != 36
            || !run_id
                .chars()
                .all(|c| c.is_ascii_hexdigit() || c == '-'))
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
        && host != "127.0.0.1" && host != "::1"
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
            System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        };
        // SAFETY: OpenProcess called with limited query rights on numeric PID.
        unsafe {
            match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                Ok(handle) => {
                    let mut code = 0u32;
                    let active = GetExitCodeProcess(handle, &mut code).is_ok()
                        && code == STILL_ACTIVE.0 as u32;
                    let _ = CloseHandle(handle);
                    if active {
                        json!({"pid": pid, "state": "exists", "responsive": null})
                    } else {
                        json!({"pid": pid, "state": "not_running", "responsive": null})
                    }
                }
                Err(_) => {
                    // Could be not found or access denied
                    json!({"pid": pid, "state": "not_running", "responsive": null})
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
    let resolved_host = if host == "localhost" { "127.0.0.1" } else { host };
    let addr_str = format!("{resolved_host}:{port}");

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

    let path = if url.path.is_empty() { "/" } else { url.path.as_str() };
    let request_data = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: agent-harness\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
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
    context.check()?;
    let (url, timeout) = validate(args)?;
    let started = Instant::now();

    let mut result = json!({
        "sampledAt": chrono::Utc::now().to_rfc3339(),
    });

    if let Some(pid) = args.pid {
        result["process"] = check_pid(pid);
    }

    if let Some(run_id) = &args.run_id {
        result["run"] = json!({
            "runId": run_id,
            "state": "unknown",
            "reason": "not_retained_in_this_harness_process"
        });
    }

    if let Some(list_filter) = args.list {
        let _ = list_filter;
        result["runs"] = json!([]);
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
        "Point-in-time observations. PID existence does not prove responsiveness or identity (PID reuse). TCP acceptance is not application health or ownership by the requested PID. HTTP health means a 2xx status only, not body validation. Run lifecycle/exit codes cover retained children started by this harness process, not descendants or external processes."
    );

    Ok(result)
}

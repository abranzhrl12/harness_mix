use super::validation::HttpUrl;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpStream,
    time::{Duration, Instant},
};

pub(super) fn probe(url: &HttpUrl, timeout: Duration) -> Value {
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
        &match addr_str.parse::<std::net::SocketAddr>() {
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

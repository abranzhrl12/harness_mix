use super::{http_probe, pid, validation};
use std::{
    io::{Read, Write},
    time::{Duration, Instant},
};

#[test]
fn health_urls_reject_tls_and_request_injection() {
    assert_eq!(
        validation::parse_health_url("https://127.0.0.1:1/health")
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
        assert!(validation::parse_health_url(url).is_err(), "{url:?}");
    }
    let parsed = validation::parse_health_url("http://[::1]:1234/health").unwrap();
    assert_eq!(parsed.host, "::1");
    assert_eq!(parsed.port, 1234);
    assert_eq!(parsed.path, "/health");
}

#[test]
fn windows_probe_errors_do_not_claim_process_absence() {
    assert_eq!(pid::windows_open_error(5), ("unknown", "access_denied"));
    assert_eq!(pid::windows_open_error(87), ("not_running", "not_found"));
    assert_eq!(pid::windows_open_error(1168), ("not_running", "not_found"));
    assert_eq!(pid::windows_open_error(12345), ("unknown", "probe_failed"));
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
    let url = validation::parse_health_url(&format!("http://[::1]:{port}/health")).unwrap();
    assert_eq!(
        http_probe::probe(&url, Duration::from_secs(1))["healthy"],
        true
    );
    server.join().unwrap();
}

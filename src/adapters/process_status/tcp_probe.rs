use serde_json::{Value, json};
use std::{
    net::{SocketAddr, TcpStream},
    time::{Duration, Instant},
};

pub(super) fn probe(host: &str, port: u16, timeout: Duration) -> Value {
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

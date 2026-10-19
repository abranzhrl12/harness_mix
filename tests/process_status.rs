use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use agent_harness::adapters;
use harness_domain::contracts::{BootstrapArgs, ProcessStatusArgs, Request};
use tempfile::tempdir;

#[test]
fn test_process_status_requires_at_least_one_mode() {
    let dir = tempdir().unwrap();
    let app = adapters::application(dir.path()).unwrap();
    let boot = app.execute(Request::Bootstrap(BootstrapArgs {
        compact: false,
        session: None,
        include_rules: None,
    }));
    let session = boot["data"]["sessionId"].as_str().unwrap().to_string();

    let res = app.execute(Request::ProcessStatus(ProcessStatusArgs {
        session: Some(session),
        pid: None,
        run_id: None,
        list: None,
        port: None,
        host: None,
        health_url: None,
        timeout_ms: None,
        limit: None,
    }));
    assert_eq!(res["state"], "error");
    assert_eq!(res["diagnostics"][0]["code"], "ARGUMENT");
}

#[test]
fn test_process_status_current_pid() {
    let dir = tempdir().unwrap();
    let app = adapters::application(dir.path()).unwrap();
    let boot = app.execute(Request::Bootstrap(BootstrapArgs {
        compact: false,
        session: None,
        include_rules: None,
    }));
    let session = boot["data"]["sessionId"].as_str().unwrap().to_string();

    let pid = std::process::id();
    let res = app.execute(Request::ProcessStatus(ProcessStatusArgs {
        session: Some(session),
        pid: Some(pid),
        run_id: None,
        list: None,
        port: None,
        host: None,
        health_url: None,
        timeout_ms: None,
        limit: None,
    }));
    assert_eq!(res["state"], "ok");
    assert_eq!(res["data"]["process"]["pid"], pid);
    assert_eq!(res["data"]["process"]["state"], "exists");
}

#[test]
fn test_process_status_dead_pid() {
    let dir = tempdir().unwrap();
    let app = adapters::application(dir.path()).unwrap();
    let boot = app.execute(Request::Bootstrap(BootstrapArgs {
        compact: false,
        session: None,
        include_rules: None,
    }));
    let session = boot["data"]["sessionId"].as_str().unwrap().to_string();

    let res = app.execute(Request::ProcessStatus(ProcessStatusArgs {
        session: Some(session),
        pid: Some(2_147_483_000),
        run_id: None,
        list: None,
        port: None,
        host: None,
        health_url: None,
        timeout_ms: None,
        limit: None,
    }));
    assert_eq!(res["state"], "ok");
    assert_eq!(res["data"]["process"]["state"], "not_running");
}

#[test]
fn test_process_status_tcp_port_probe() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let port = listener.local_addr().unwrap().port();

    let dir = tempdir().unwrap();
    let app = adapters::application(dir.path()).unwrap();
    let boot = app.execute(Request::Bootstrap(BootstrapArgs {
        compact: false,
        session: None,
        include_rules: None,
    }));
    let session = boot["data"]["sessionId"].as_str().unwrap().to_string();

    let res = app.execute(Request::ProcessStatus(ProcessStatusArgs {
        session: Some(session),
        pid: None,
        run_id: None,
        list: None,
        port: Some(port),
        host: Some("127.0.0.1".into()),
        health_url: None,
        timeout_ms: Some(1000),
        limit: None,
    }));
    assert_eq!(res["state"], "ok");
    assert_eq!(res["data"]["tcp"]["accepting"], true);
    assert_eq!(res["data"]["tcp"]["state"], "accepting");
    assert_eq!(res["data"]["tcp"]["port"], port);
    assert_eq!(res["data"]["tcp"]["host"], "127.0.0.1");
}

#[test]
fn test_process_status_http_health_url() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
    let port = listener.local_addr().unwrap().port();

    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"status\":\"ok\"}";
            let _ = stream.write_all(response.as_bytes());
        }
    });

    thread::sleep(Duration::from_millis(50));

    let dir = tempdir().unwrap();
    let app = adapters::application(dir.path()).unwrap();
    let boot = app.execute(Request::Bootstrap(BootstrapArgs {
        compact: false,
        session: None,
        include_rules: None,
    }));
    let session = boot["data"]["sessionId"].as_str().unwrap().to_string();

    let res = app.execute(Request::ProcessStatus(ProcessStatusArgs {
        session: Some(session),
        pid: None,
        run_id: None,
        list: None,
        port: None,
        host: None,
        health_url: Some(format!("http://127.0.0.1:{port}/healthz")),
        timeout_ms: Some(1500),
        limit: None,
    }));
    assert_eq!(res["state"], "ok");
    assert_eq!(res["data"]["health"]["healthy"], true);
    assert_eq!(res["data"]["health"]["responding"], true);
    assert_eq!(res["data"]["health"]["statusCode"], 200);
}

#[test]
fn test_process_status_rejects_non_loopback() {
    let dir = tempdir().unwrap();
    let app = adapters::application(dir.path()).unwrap();
    let boot = app.execute(Request::Bootstrap(BootstrapArgs {
        compact: false,
        session: None,
        include_rules: None,
    }));
    let session = boot["data"]["sessionId"].as_str().unwrap().to_string();

    let res = app.execute(Request::ProcessStatus(ProcessStatusArgs {
        session: Some(session),
        pid: None,
        run_id: None,
        list: None,
        port: None,
        host: None,
        health_url: Some("http://google.com/health".into()),
        timeout_ms: None,
        limit: None,
    }));
    assert_eq!(res["state"], "error");
    assert_eq!(res["diagnostics"][0]["code"], "ARGUMENT");
}

#[test]
fn https_is_rejected_before_any_plaintext_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let args:ProcessStatusArgs=serde_json::from_value(serde_json::json!({"healthUrl":format!("https://127.0.0.1:{}/health",listener.local_addr().unwrap().port())})).unwrap();
    let error =
        adapters::process_status::handle_process_status(&args, &Default::default()).unwrap_err();
    assert_eq!(error.0, "UNSUPPORTED");
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn standalone_run_queries_do_not_fake_empty_history() {
    let args: ProcessStatusArgs =
        serde_json::from_value(serde_json::json!({"list":"recent"})).unwrap();
    assert_eq!(
        adapters::process_status::handle_process_status(&args, &Default::default())
            .unwrap_err()
            .0,
        "UNSUPPORTED"
    );
}

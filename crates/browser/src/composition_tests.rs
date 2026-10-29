use harness_application::ports::browser::BrowserInspection;
use harness_domain::{
    browser::{BrowserArgs, BrowserResult},
    execution::OperationContext,
};

use super::BrowserService;

#[test]
#[ignore = "launches an installed Chromium browser with an isolated profile"]
fn smoke_auto_launch_captures_errors_without_manual_debug_port() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant},
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = stop.clone();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !server_stop.load(Ordering::Relaxed) && Instant::now() < deadline {
            if let Ok((mut stream, _)) = listener.accept() {
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let _ = stream.read(&mut [0; 4096]);
                let body = "<script>throw new Error('harness-initial-load-regression')</script><button id='repro' onclick=\"throw new TypeError('harness-auto-launch-regression')\">Reproduce</button>";
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            } else {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    });
    let service = BrowserService::new();
    let context = OperationContext::default();
    let session = Some("auto-launch-smoke".to_owned());
    let connected = service.browser(
        &BrowserArgs::Connect {
            endpoint: "auto".into(),
            tab: Some(url.clone()),
            session: session.clone(),
        },
        &context,
    );
    let connected = match connected {
        Ok(result) => result,
        Err(error) => {
            stop.store(true, Ordering::Relaxed);
            server.join().unwrap();
            panic!("automatic connect failed: {error:?}");
        }
    };
    let BrowserResult::Connected {
        connection_id,
        selected_tab: Some(tab),
        ..
    } = connected
    else {
        panic!("expected automatically attached application tab")
    };
    assert_eq!(tab.url, url);
    service
        .browser(
            &BrowserArgs::Click {
                connection_id: connection_id.clone(),
                target_id: tab.target_id.clone(),
                selector: "#repro".into(),
                session: session.clone(),
            },
            &context,
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let captured = loop {
        let result = service
            .browser(
                &BrowserArgs::Errors {
                    connection_id: connection_id.clone(),
                    target_id: Some(tab.target_id.clone()),
                    after: None,
                    limit: Some(256),
                    session: session.clone(),
                },
                &context,
            )
            .unwrap();
        if let BrowserResult::Errors { events, .. } = result {
            let has_exception = |needle: &str| {
                events.iter().any(|event| {
                matches!(&event.event, harness_domain::browser::BrowserEvent::Exception { message, .. } if message.contains(needle))
            })
            };
            if has_exception("harness-auto-launch-regression")
                && has_exception("harness-initial-load-regression")
            {
                break true;
            }
        }
        if Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    service
        .browser(
            &BrowserArgs::Disconnect {
                connection_id,
                session,
            },
            &context,
        )
        .unwrap();
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();
    assert!(
        captured,
        "uncaught exception must be captured without manual attach or PowerShell setup"
    );
}

#[test]
#[ignore = "requires an already-running local Chrome; set HARNESS_BROWSER_TEST_ENDPOINT to its direct CDP WebSocket"]
fn smoke_connect_targets_owner_isolation_and_disconnect() {
    let endpoint =
        std::env::var("HARNESS_BROWSER_TEST_ENDPOINT").expect("set direct CDP websocket endpoint");
    let service = BrowserService::new();
    let context = OperationContext::default();
    let connected = service
        .browser(
            &BrowserArgs::Connect {
                endpoint: endpoint.clone(),
                tab: None,
                session: Some("smoke-owner".into()),
            },
            &context,
        )
        .unwrap();
    assert!(matches!(connected, BrowserResult::Connected { .. }));

    // Reconnecting with the same session replaces the prior connection seamlessly (#8)
    let reconnected = service
        .browser(
            &BrowserArgs::Connect {
                endpoint,
                tab: None,
                session: Some("smoke-owner".into()),
            },
            &context,
        )
        .unwrap();
    let BrowserResult::Connected { connection_id, .. } = reconnected else {
        panic!("unexpected connect result");
    };
    let listed = service
        .browser(
            &BrowserArgs::Targets {
                connection_id: connection_id.clone(),
                tab: None,
                session: Some("smoke-owner".into()),
            },
            &context,
        )
        .unwrap();
    let BrowserResult::Targets { targets, .. } = listed else {
        panic!("unexpected targets result");
    };
    assert!(!targets.is_empty(), "expected at least one page target");
    let first_target_id = targets[0].id.clone();

    let attached = service
        .browser(
            &BrowserArgs::Attach {
                connection_id: connection_id.clone(),
                target_id: first_target_id.clone(),
                session: Some("smoke-owner".into()),
            },
            &context,
        )
        .unwrap();
    assert!(matches!(attached, BrowserResult::Attached { .. }));

    let inspected = service
        .browser(
            &BrowserArgs::Inspect {
                connection_id: connection_id.clone(),
                target_id: first_target_id.clone(),
                selector: None,
                max_chars: Some(2000),
                session: Some("smoke-owner".into()),
            },
            &context,
        )
        .unwrap();
    assert!(matches!(inspected, BrowserResult::Inspected { .. }));

    let screenshot = service
        .browser(
            &BrowserArgs::Screenshot {
                connection_id: connection_id.clone(),
                target_id: first_target_id,
                selector: None,
                session: Some("smoke-owner".into()),
            },
            &context,
        )
        .unwrap();
    assert!(matches!(screenshot, BrowserResult::Screenshot { .. }));
    let denied = service.browser(
        &BrowserArgs::Targets {
            connection_id: connection_id.clone(),
            tab: None,
            session: Some("other-owner".into()),
        },
        &context,
    );
    assert_eq!(denied.unwrap_err().0, "BROWSER_SESSION");
    let disconnected = service
        .browser(
            &BrowserArgs::Disconnect {
                connection_id,
                session: Some("smoke-owner".into()),
            },
            &context,
        )
        .unwrap();
    assert!(matches!(disconnected, BrowserResult::Disconnected { .. }));
}

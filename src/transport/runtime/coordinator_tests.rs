
use super::run_server;
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[tokio::test]
async fn wait_and_drain_failures_are_both_preserved() {
    let result = run_server(
        Ok::<_, io::Error>(()),
        |_| async { Err::<(), _>(io::Error::other("service wait failed")) },
        async { Err("worker drain timed out".to_owned()) },
    )
    .await;
    assert_eq!(
        result.unwrap_err().to_string(),
        "service wait failed; worker drain timed out"
    );
}

#[tokio::test]
async fn wait_failure_still_drains_workers() {
    let drained = Arc::new(AtomicBool::new(false));
    let signal = drained.clone();
    let result = run_server(
        Ok::<_, io::Error>(()),
        |_| async { Err::<(), _>(io::Error::other("service wait failed")) },
        async move {
            signal.store(true, Ordering::SeqCst);
            Ok(())
        },
    )
    .await;
    assert!(drained.load(Ordering::SeqCst));
    assert_eq!(result.unwrap_err().to_string(), "service wait failed");
}

#[tokio::test]
async fn startup_failure_drains_workers_and_keeps_handshake_error() {
    let drained = Arc::new(AtomicBool::new(false));
    let drain_signal = Arc::clone(&drained);
    let result = run_server(
        Err::<(), _>(io::Error::other("handshake failed")),
        |_| async { Ok::<(), io::Error>(()) },
        async move {
            drain_signal.store(true, Ordering::SeqCst);
            Ok(())
        },
    )
    .await;

    assert!(drained.load(Ordering::SeqCst));
    assert_eq!(result.unwrap_err().to_string(), "handshake failed");
}

#[tokio::test]
async fn startup_and_drain_failures_are_combined() {
    let result = run_server(
        Err::<(), _>(io::Error::other("handshake failed")),
        |_| async { Ok::<(), io::Error>(()) },
        async { Err("worker drain timed out".to_owned()) },
    )
    .await;

    assert_eq!(
        result.unwrap_err().to_string(),
        "handshake failed; worker drain timed out"
    );
}

#[tokio::test]
async fn drain_failure_is_reported_after_service_wait() {
    let result = run_server(
        Ok::<_, io::Error>(()),
        |_| async { Ok::<(), io::Error>(()) },
        async { Err("worker drain timed out".to_owned()) },
    )
    .await;

    assert_eq!(result.unwrap_err().to_string(), "worker drain timed out");
}

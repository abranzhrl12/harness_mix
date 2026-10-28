use harness_application::ports::browser::BrowserInspection;
use harness_domain::{
    browser::{BrowserArgs, BrowserResult},
    execution::OperationContext,
};

use super::BrowserService;

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

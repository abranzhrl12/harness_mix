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
                endpoint,
                session: Some("smoke-owner".into()),
            },
            &context,
        )
        .unwrap();
    let BrowserResult::Connected { connection_id } = connected else {
        panic!("unexpected connect result");
    };
    let listed = service
        .browser(
            &BrowserArgs::Targets {
                connection_id: connection_id.clone(),
                session: Some("smoke-owner".into()),
            },
            &context,
        )
        .unwrap();
    assert!(matches!(listed, BrowserResult::Targets { .. }));
    let denied = service.browser(
        &BrowserArgs::Targets {
            connection_id: connection_id.clone(),
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

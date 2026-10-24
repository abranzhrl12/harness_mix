use super::CdpBrowserBackend;

#[test]
fn constructing_the_backend_does_not_start_a_runtime() {
    let backend = CdpBrowserBackend::new();
    assert!(backend.actor.lock().unwrap().is_none());
}

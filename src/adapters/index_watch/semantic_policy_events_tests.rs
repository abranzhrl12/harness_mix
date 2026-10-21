//! Regression cases for configuration events that schedule reconciliation.
use super::{events::relevant, pending::Pending};
use notify::{Event, EventKind};

#[test]
fn harness_configuration_changes_schedule_reconciliation() {
    let root = std::path::PathBuf::from("workspace");
    let event = Event::new(EventKind::Modify(notify::event::ModifyKind::Any))
        .add_path(root.join(".agents/harness.config.json"));
    let mut pending = Pending::default();
    relevant(&root, event, &mut pending);
    assert!(pending.full);
    assert!(pending.first.is_some());
}

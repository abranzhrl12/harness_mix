use harness_domain::jobs::{JobLifecycle, JobStatus, TransitionError};

#[test]
fn status_names_match_the_persisted_and_public_json_values() {
    let statuses = [
        (JobStatus::Queued, "queued"),
        (JobStatus::Running, "running"),
        (JobStatus::RetryWait, "retry_wait"),
        (JobStatus::Succeeded, "succeeded"),
        (JobStatus::Failed, "failed"),
        (JobStatus::Cancelled, "cancelled"),
        (JobStatus::TimedOut, "timed_out"),
        (JobStatus::Interrupted, "interrupted"),
    ];

    for (status, name) in statuses {
        assert_eq!(status.as_str(), name);
        assert_eq!(serde_json::to_value(status).unwrap(), name);
        assert_eq!(
            serde_json::from_value::<JobStatus>(name.into()).unwrap(),
            status
        );
        assert_eq!(JobStatus::parse(name), Some(status));
    }
    assert_eq!(JobStatus::parse("unknown"), None);
}

#[test]
fn lifecycle_tracks_first_and_retried_attempts_then_completion() {
    let mut lifecycle = JobLifecycle::queued();
    assert_eq!(lifecycle.attempt(), 0);
    lifecycle.transition(JobStatus::Running, 1).unwrap();
    lifecycle.transition(JobStatus::RetryWait, 1).unwrap();
    lifecycle.transition(JobStatus::Running, 2).unwrap();
    lifecycle.transition(JobStatus::Succeeded, 2).unwrap();

    assert_eq!(lifecycle.status(), JobStatus::Succeeded);
    assert_eq!(lifecycle.attempt(), 2);
    assert!(lifecycle.status().is_terminal());
    assert_eq!(
        lifecycle.transition(JobStatus::Running, 3),
        Err(TransitionError::InvalidTransition)
    );
}

#[test]
fn plans_can_fail_before_the_first_launch_or_after_a_retry_wait() {
    let mut initial = JobLifecycle::queued();
    initial.transition(JobStatus::Failed, 1).unwrap();

    let mut retry = JobLifecycle::queued();
    retry.transition(JobStatus::Running, 1).unwrap();
    retry.transition(JobStatus::RetryWait, 1).unwrap();
    retry.transition(JobStatus::Failed, 2).unwrap();

    assert_eq!(initial.status(), JobStatus::Failed);
    assert_eq!(retry.status(), JobStatus::Failed);
    assert_eq!(retry.attempt(), 2);
}

#[test]
fn lifecycle_allows_cancellation_and_recovery_from_active_states() {
    let mut queued = JobLifecycle::queued();
    queued.transition(JobStatus::Cancelled, 0).unwrap();

    let mut running = JobLifecycle::queued();
    running.transition(JobStatus::Running, 1).unwrap();
    assert!(running.interrupt_for_recovery());

    let mut waiting = JobLifecycle::queued();
    waiting.transition(JobStatus::Running, 1).unwrap();
    waiting.transition(JobStatus::RetryWait, 1).unwrap();
    waiting.transition(JobStatus::Cancelled, 1).unwrap();

    assert_eq!(queued.status(), JobStatus::Cancelled);
    assert_eq!(running.status(), JobStatus::Interrupted);
    assert_eq!(waiting.status(), JobStatus::Cancelled);
}

#[test]
fn transition_rejects_skipped_attempts_and_unknown_edges_without_mutating() {
    let mut lifecycle = JobLifecycle::queued();
    assert_eq!(
        lifecycle.transition(JobStatus::Running, 2),
        Err(TransitionError::InvalidAttempt)
    );
    assert_eq!(
        lifecycle.transition(JobStatus::Succeeded, 0),
        Err(TransitionError::InvalidTransition)
    );
    assert_eq!(lifecycle, JobLifecycle::queued());
}

#[test]
fn restore_keeps_the_existing_snapshot_acceptance_rules() {
    for name in [
        "queued",
        "running",
        "retry_wait",
        "succeeded",
        "failed",
        "cancelled",
        "timed_out",
        "interrupted",
    ] {
        assert_eq!(
            JobLifecycle::restore(name, 0, 1).unwrap().status().as_str(),
            name
        );
        assert_eq!(JobLifecycle::restore(name, 2, 3).unwrap().attempt(), 2);
    }
    assert!(JobLifecycle::restore("unknown", 0, 1).is_none());
    assert!(JobLifecycle::restore("running", 2, 1).is_none());
    assert!(JobLifecycle::restore("running", 4, 3).is_none());
    assert!(JobLifecycle::restore("running", 0, 0).is_none());
}

#[test]
fn recovery_interrupts_any_accepted_active_snapshot_and_preserves_its_attempt() {
    for (status, attempt) in [("queued", 2), ("running", 0), ("retry_wait", 1)] {
        let mut lifecycle = JobLifecycle::restore(status, attempt, 3).unwrap();
        assert!(lifecycle.interrupt_for_recovery());
        assert_eq!(lifecycle.status(), JobStatus::Interrupted);
        assert_eq!(lifecycle.attempt(), attempt);
    }

    let mut terminal = JobLifecycle::restore("succeeded", 1, 3).unwrap();
    assert!(!terminal.interrupt_for_recovery());
    assert_eq!(terminal.status(), JobStatus::Succeeded);
}

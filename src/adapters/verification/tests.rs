use super::*;

include!("tests/dotnet.rs");
include!("tests/javascript.rs");
include!("tests/angular.rs");
include!("tests/python.rs");
include!("tests/go.rs");
include!("tests/rust.rs");

#[test]
fn verification_status_interprets_exit_and_stops_consistently() {
    use harness_domain::process::{ProcessOutcome, StopReason};
    use std::time::Duration;

    let outcome = |exit_code, reason| ProcessOutcome {
        exit_code,
        stdout: vec![],
        stderr: vec![],
        elapsed: Duration::ZERO,
        reason,
        tree_termination_confirmed: true,
    };

    let success = VerificationStatus::interpret(&outcome(Some(0), StopReason::Exited));
    assert_eq!(success, VerificationStatus { passed: true, limited: false, cancelled: false, timed_out: false });
    assert!(success.complete());

    let failure = VerificationStatus::interpret(&outcome(Some(1), StopReason::Exited));
    assert!(!failure.passed);
    assert!(failure.complete());

    let stopped = [StopReason::OutputLimit, StopReason::Cancelled, StopReason::TimedOut];
    for reason in stopped {
        let status = VerificationStatus::interpret(&outcome(None, reason));
        assert!(!status.passed);
        assert!(!status.complete());
    }
}

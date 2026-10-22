use harness_application::{
    indexing::{file, transaction},
    ports::indexing::{FileIndexPort, FileTransactionPort},
};
use harness_domain::{Failure, execution::OperationContext};
use serde_json::{Value, json};
use std::{cell::RefCell, rc::Rc};

const FAILED: Failure = ("TEST", "Injected failure");
const UNCERTAIN: Failure = ("SYNC_COMMIT_UNCERTAIN", "Unknown durability");
#[derive(Clone, Default)]
struct Fake {
    log: Rc<RefCell<Vec<&'static str>>>,
    fail: Option<&'static str>,
    cancel: Option<&'static str>,
    rollback_fails: bool,
    commit_context: Option<OperationContext>,
}
impl Fake {
    fn step(&self, name: &'static str, context: &OperationContext) -> Result<(), Failure> {
        self.log.borrow_mut().push(name);
        if self.cancel == Some(name) {
            context.cancel();
        }
        if self.fail == Some(name) {
            Err(FAILED)
        } else {
            Ok(())
        }
    }
}
impl FileTransactionPort for Fake {
    type Publication = u8;
    type Output = Value;
    fn validate(&mut self, c: &OperationContext) -> Result<(), Failure> {
        self.step("validate", c)
    }
    fn publish(&mut self, c: &OperationContext) -> Result<u8, Failure> {
        self.step("publish", c)?;
        Ok(42)
    }
    fn recheck(&mut self, p: &u8, c: &OperationContext) -> Result<(), Failure> {
        assert_eq!(*p, 42);
        self.step("recheck", c)
    }
    fn integrity(&mut self, p: &u8, c: &OperationContext) -> Result<(), Failure> {
        assert_eq!(*p, 42);
        self.step("integrity", c)
    }
    fn rollback(self) -> Result<(), Failure> {
        self.log.borrow_mut().push("rollback");
        if self.rollback_fails {
            Err(UNCERTAIN)
        } else {
            Ok(())
        }
    }
    fn commit(self, p: u8) -> Result<Value, Failure> {
        assert_eq!(p, 42);
        self.log.borrow_mut().push("commit");
        if let Some(context) = self.commit_context {
            context.cancel();
        }
        if self.fail == Some("commit") {
            Err(UNCERTAIN)
        } else {
            Ok(json!({"committed":true}))
        }
    }
}
impl FileIndexPort for Fake {
    type Transaction<'a> = Fake;
    fn validate_request(&mut self, c: &OperationContext) -> Result<(), Failure> {
        self.step("request", c)
    }
    fn load_scope(&mut self, c: &OperationContext) -> Result<(), Failure> {
        self.step("scope", c)
    }
    fn read_source(&mut self, c: &OperationContext) -> Result<(), Failure> {
        self.step("source", c)
    }
    fn begin(&mut self, c: &OperationContext) -> Result<Fake, Failure> {
        self.step("begin", c)?;
        Ok(self.clone())
    }
}
#[test]
fn full_file_lifecycle_orders_preflight_transaction_and_commit() {
    let mut f = Fake::default();
    assert_eq!(
        file::sync_file(&mut f, &OperationContext::default()).unwrap()["committed"],
        true
    );
    assert_eq!(
        *f.log.borrow(),
        [
            "request",
            "scope",
            "source",
            "begin",
            "validate",
            "publish",
            "recheck",
            "integrity",
            "commit"
        ]
    );
}
#[test]
fn failure_in_each_preflight_stage_never_starts_later_stages() {
    let stages = ["request", "scope", "source", "begin"];
    for (i, stage) in stages.iter().enumerate() {
        let mut f = Fake {
            fail: Some(stage),
            ..Fake::default()
        };
        assert_eq!(
            file::sync_file(&mut f, &OperationContext::default()).unwrap_err(),
            FAILED
        );
        assert_eq!(*f.log.borrow(), stages[..=i]);
    }
}
#[test]
fn cancellation_between_each_preflight_stage_stops_and_opened_tx_rolls_back() {
    let stages = ["request", "scope", "source", "begin"];
    for (i, stage) in stages.iter().enumerate() {
        let mut f = Fake {
            cancel: Some(stage),
            ..Fake::default()
        };
        assert_eq!(
            file::sync_file(&mut f, &OperationContext::default())
                .unwrap_err()
                .0,
            "CANCELLED"
        );
        let mut expected = stages[..=i].to_vec();
        if *stage == "begin" {
            expected.push("rollback");
        }
        assert_eq!(*f.log.borrow(), expected);
    }
}
#[test]
fn every_transaction_failure_rolls_back_and_never_commits() {
    let stages = ["validate", "publish", "recheck", "integrity"];
    for (i, stage) in stages.iter().enumerate() {
        let f = Fake {
            fail: Some(stage),
            ..Fake::default()
        };
        let log = f.log.clone();
        assert_eq!(
            transaction::execute(f, &OperationContext::default()).unwrap_err(),
            FAILED
        );
        let mut expected = stages[..=i].to_vec();
        expected.push("rollback");
        assert_eq!(*log.borrow(), expected);
    }
}
#[test]
fn cancellation_after_every_transaction_stage_rolls_back_even_after_integrity() {
    for stage in ["validate", "publish", "recheck", "integrity"] {
        let f = Fake {
            cancel: Some(stage),
            ..Fake::default()
        };
        let log = f.log.clone();
        assert_eq!(
            transaction::execute(f, &OperationContext::default())
                .unwrap_err()
                .0,
            "CANCELLED"
        );
        assert_eq!(log.borrow().last(), Some(&"rollback"));
        assert!(!log.borrow().contains(&"commit"));
    }
}
#[test]
fn rollback_failure_takes_precedence_over_failure_and_cancellation() {
    let f = Fake {
        fail: Some("publish"),
        cancel: Some("publish"),
        rollback_fails: true,
        ..Fake::default()
    };
    assert_eq!(
        transaction::execute(f, &OperationContext::default()).unwrap_err(),
        UNCERTAIN
    );
}
#[test]
fn commit_failure_is_uncertain_not_a_fake_rollback_success() {
    let f = Fake {
        fail: Some("commit"),
        ..Fake::default()
    };
    let log = f.log.clone();
    assert_eq!(
        transaction::execute(f, &OperationContext::default()).unwrap_err(),
        UNCERTAIN
    );
    assert_eq!(log.borrow().last(), Some(&"commit"));
    assert!(!log.borrow().contains(&"rollback"));
}
#[test]
fn already_cancelled_transaction_is_rolled_back_without_validating() {
    let f = Fake::default();
    let log = f.log.clone();
    let context = OperationContext::default();
    context.cancel();
    assert_eq!(
        transaction::execute(f, &context).unwrap_err().0,
        "CANCELLED"
    );
    assert_eq!(*log.borrow(), ["rollback"]);
}

#[test]
fn cancellation_at_commit_point_does_not_turn_confirmed_commit_into_failure() {
    let context = OperationContext::default();
    let f = Fake {
        commit_context: Some(context.clone()),
        ..Fake::default()
    };
    assert_eq!(
        transaction::execute(f, &context).unwrap()["committed"],
        true
    );
    assert_eq!(context.check().unwrap_err().0, "CANCELLED");
}

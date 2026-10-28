pub use harness_domain::Failure;
#[path = "../../../../crates/domain/src/editing/batch.rs"]
mod batch;
#[allow(dead_code)]
#[path = "../../../../crates/domain/src/editing/matching.rs"]
mod matching;
fn main() {
    assert!(!batch::eligible(0, 0, || 0));
    let needles = [
        std::borrow::Cow::Borrowed("one"),
        std::borrow::Cow::Borrowed("two"),
    ];
    batch::locate_chunks("one\ntwo", &needles).unwrap();
}

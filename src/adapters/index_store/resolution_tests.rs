use super::*;

#[test]
fn retry_coverage_is_bounded_and_competing_commits_invalidate_it() {
    let mut memo = RetryMemo {
        enabled: true,
        ..Default::default()
    };
    memo.observe_version(1);
    memo.remember(["already_checked".to_owned()]);
    memo.observe_version(1);
    assert!(memo.json().contains("already_checked"));
    memo.observe_version(2);
    assert_eq!(memo.json(), "[]");
    memo.remember((0..10_000).map(|index| format!("name_{index}")));
    assert!(memo.names.len() <= 4096);
    assert!(memo.bytes <= 65536);
    memo.clear();
    assert_eq!(memo.json(), "[]");
}

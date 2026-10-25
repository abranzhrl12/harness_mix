use super::*;
use crate::adapters::tasks::TaskManager;

async fn terminal_status(manager: &TaskManager, id: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let args = serde_json::from_value(json!({"action":"status","taskId":id})).unwrap();
            let value = manager.handle(args).await.unwrap();
            if matches!(
                value["status"].as_str(),
                Some("succeeded" | "failed" | "cancelled" | "timed_out")
            ) {
                return value;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn native_typescript_jobs_parse_syntax_without_executing_code() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("valid.tsx"), "const value: number = 1; const view = <main>{value}</main>; throw new Error('must not execute');").unwrap();
    std::fs::write(root.path().join("invalid.ts"), "const value: = ;").unwrap();
    let manager = TaskManager::new(root.path()).unwrap();
    for (scope, expected) in [("valid.tsx", "succeeded"), ("invalid.ts", "failed")] {
        let args = serde_json::from_value(json!({"action":"start","verify":{"language":"typescript","scope":scope,"action":"build"}})).unwrap();
        let queued = manager.handle(args).await.unwrap();
        let state = terminal_status(&manager, queued["taskId"].as_str().unwrap()).await;
        assert_eq!(state["status"], expected);
        assert_eq!(
            state["result"]["verificationMode"],
            "typescript_syntax_only"
        );
        assert_eq!(state["result"]["codeExecuted"], false);
    }
    let args = serde_json::from_value(json!({"action":"start","verify":{"language":"typescript","scope":"valid.tsx","action":"test"}})).unwrap();
    assert_eq!(manager.handle(args).await.unwrap_err().0, "UNSUPPORTED");
    assert!(manager.shutdown_and_wait(Duration::from_secs(2)).await);
}

#[test]
fn native_preparation_revalidates_workspace_manifest() {
    let root = tempfile::tempdir().unwrap();
    let manifest = root.path().join("Cargo.toml");
    std::fs::write(&manifest, "[package]\nname='fixture'\nversion='0.1.0'\n").unwrap();
    let args = serde_json::from_value(json!({"language":"rust"})).unwrap();
    assert!(
        NativeVerificationExecutor
            .prepare(root.path(), &args, None)
            .is_ok()
    );
    std::fs::remove_file(&manifest).unwrap();
    let error = NativeVerificationExecutor
        .prepare(root.path(), &args, None)
        .err()
        .unwrap();
    assert!(matches!(error.0, "VERIFY_SCOPE" | "PATH"));
}

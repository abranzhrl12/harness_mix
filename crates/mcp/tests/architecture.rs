use harness_application::{Application, ports::{SessionContext, SessionRepository}};
use harness_application::ports::browser::BrowserInspection;
use harness_application::ports::workspace::{
    ArtifactQueries, LogPersistence, SessionPersistence, SourceEditing, SourceQueries, SqlAccess,
    Verification, VersionControl, WorkspaceDiagnostics, WorkspaceMetadata,
};
use harness_domain::{Failure, execution::OperationContext};
use harness_mcp::{BoundedLines, MAX_MESSAGE_BYTES, McpServer};
use rmcp::ServerHandler;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

struct TestWorkspace;

impl WorkspaceMetadata for TestWorkspace {
    fn info(&self) -> Value { json!({}) }
    fn rules(&self, _: &OperationContext) -> Result<String, Failure> { Ok(String::new()) }
    fn capabilities(&self) -> Value { json!({}) }
}
impl BrowserInspection for TestWorkspace {}
impl SourceQueries for TestWorkspace {
    fn read(
        &self,
        _: &harness_domain::contracts::ReadArgs,
        _: &OperationContext,
    ) -> Result<Value, Failure> { unreachable!() }
    fn search(
        &self,
        _: &harness_domain::contracts::SearchArgs,
        _: &OperationContext,
    ) -> Result<Value, Failure> { unreachable!() }
}
impl SourceEditing for TestWorkspace {
    fn replace(
        &self,
        _: &harness_domain::contracts::ReplaceArgs,
        _: &OperationContext,
    ) -> Result<Value, Failure> { unreachable!() }
}
impl WorkspaceDiagnostics for TestWorkspace {}
impl VersionControl for TestWorkspace {}
impl SqlAccess for TestWorkspace {}
impl ArtifactQueries for TestWorkspace {}
impl SessionPersistence for TestWorkspace {}
impl LogPersistence for TestWorkspace {}
impl Verification for TestWorkspace {}

struct TestSessions;
impl SessionRepository for TestSessions {
    fn workflow_gate(&mut self, _: &str) -> Result<Arc<Mutex<()>>, Failure> {
        Ok(Arc::new(Mutex::new(())))
    }
    fn acquire(&mut self, _: &str) -> Result<(), Failure> { Ok(()) }
    fn release(&mut self, _: &str) {}
    fn open(&mut self, _: Option<&str>) -> Result<SessionContext, Failure> { unreachable!() }
    fn update_hashes(&mut self, _: &str, _: String, _: String) -> Result<(), Failure> {
        unreachable!()
    }
}

#[test]
fn public_api_builds_a_tool_server_without_native_composition() {
    let application = Application::new(TestWorkspace, TestSessions);
    let server = McpServer::new(application, None)
        .with_eager_tools(false)
        .with_context_presentation(true);

    // Exercise the public ServerHandler implementation without native adapters.
    let _config = ServerHandler::get_info(&server);

    let _bounded_input = BoundedLines::new(tokio::io::empty(), MAX_MESSAGE_BYTES);
    assert!(MAX_MESSAGE_BYTES > 0);
}

fn collect_rust_files(directory: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn contains_native_runtime_source_dependency(source: &str) -> bool {
    [
        "std::env",
        "env::var",
        "NativeApplication",
        "adapters::tasks",
        "harness_cli",
        // Public MCP route names are not runtime dependencies. Reject Rust
        // module paths that couple this crate to a native browser layer.
        "harness_browser::",
    ]
    .iter()
    .any(|forbidden| source.contains(forbidden))
}

#[test]
fn architecture_guard_distinguishes_browser_route_names_from_dependencies() {
    assert!(!contains_native_runtime_source_dependency(
        "async fn harness_browser() {}"
    ));
    assert!(contains_native_runtime_source_dependency(
        "use harness_browser::Browser;"
    ));
}

#[test]
fn production_mcp_layer_has_no_native_runtime_dependencies_or_environment_reads() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    for forbidden in [
        "harness-cli",
        "harness-browser",
        "notify =",
        "rusqlite",
        "tiberius",
        "native-tls",
        "mimalloc",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "MCP crate directly depends on native runtime component {forbidden}"
        );
    }

    let mut files = Vec::new();
    collect_rust_files(&root.join("src"), &mut files);
    assert!(!files.is_empty(), "MCP production source is missing");
    for path in files {
        let source = std::fs::read_to_string(&path).unwrap();
        assert!(
            !contains_native_runtime_source_dependency(&source),
            "{} depends on a native runtime detail",
            path.display()
        );
    }
}

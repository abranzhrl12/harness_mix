use super::*;
use harness_domain::diagram::{DiagramEdge, DiagramFormat, DiagramNode};

#[tokio::test]
async fn renders_real_png_with_d2_when_binary_is_available() {
    let host = D2Host::default();
    if tokio::process::Command::new(&host.executable).arg("--version").output().await.is_err() {
        eprintln!("D2 rendering check skipped: d2 is not on PATH");
        return;
    }
    let args = DiagramArgs {
        nodes: vec![DiagramNode { id: "web".into(), label: "Web \"app\" \\ backend".into() }, DiagramNode { id: "db".into(), label: "Database".into() }],
        edges: vec![DiagramEdge { from: "web".into(), to: "db".into(), label: Some("queries".into()) }],
        format: DiagramFormat::Png,
    };
    let png = render(args, &host).await.expect("D2 renders a PNG");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
}

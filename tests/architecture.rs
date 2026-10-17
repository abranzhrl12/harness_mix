use serde_json::Value;
use std::process::Command;

#[test]
fn portable_layers_cannot_gain_unreviewed_infrastructure_dependencies() {
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--offline",
            "--locked",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();
    let packages = metadata["packages"].as_array().unwrap();
    for (name, allowed) in [
        (
            "harness-domain",
            &["serde", "serde_json", "sha2", "uuid", "schemars"][..],
        ),
        ("harness-application", &["harness-domain", "serde_json"][..]),
    ] {
        let package = packages
            .iter()
            .find(|package| package["name"] == name)
            .unwrap();
        for dependency in package["dependencies"].as_array().unwrap() {
            if dependency["kind"] == "dev" {
                continue;
            }
            let dependency = dependency["name"].as_str().unwrap();
            assert!(
                allowed.contains(&dependency),
                "{name} depends on {dependency}; infrastructure belongs in native adapters, not portable layers"
            );
        }
    }
}

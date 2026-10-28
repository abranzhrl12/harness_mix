use harness_skills::{
    installation::{Installation, uninstall},
    project_support::ProjectSupport,
};
use std::{fs, path::Path};

const REQUIRED_FILES: &[&str] = &[
    "systematic-debugging/SKILL.md",
    "systematic-debugging/root-cause-tracing.md",
    "systematic-debugging/defense-in-depth.md",
    "systematic-debugging/condition-based-waiting.md",
    "systematic-debugging/condition-based-waiting-example.ts",
    "systematic-debugging/find-polluter.sh",
    "systematic-debugging/LICENSE",
    "test-driven-development/SKILL.md",
    "test-driven-development/writing-good-tests.md",
    "test-driven-development/LICENSE",
    "verification-before-completion/SKILL.md",
    "verification-before-completion/LICENSE",
    "improve-codebase-architecture/SKILL.md",
    "improve-codebase-architecture/HTML-REPORT.md",
    "improve-codebase-architecture/agents/openai.yaml",
    "improve-codebase-architecture/LICENSE",
    "codebase-design/SKILL.md",
    "codebase-design/DEEPENING.md",
    "codebase-design/DESIGN-IT-TWICE.md",
    "codebase-design/agents/openai.yaml",
    "codebase-design/LICENSE",
    "domain-modeling/SKILL.md",
    "domain-modeling/LICENSE",
    "grilling/SKILL.md",
    "grilling/LICENSE",
];

fn assert_workflow_bundle(root: &Path) {
    let files = &serde_json::from_slice::<serde_json::Value>(
        &fs::read(root.join(".agents/skills/.harness-manifest.json")).unwrap(),
    )
    .unwrap()["files"];
    for relative in REQUIRED_FILES {
        let path = format!(".agents/skills/{relative}");
        let bytes = fs::read(root.join(&path)).unwrap_or_else(|error| panic!("{path}: {error}"));
        assert!(!bytes.is_empty(), "{path} must not be empty");
        assert_eq!(
            files[&path],
            harness_domain::hash(&bytes),
            "{path} must be managed"
        );
    }
}

#[test]
fn init_installs_complete_workflow_bundle_and_uninstalls_owned_support_files() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let executable = root.join("test-harness.exe");
    fs::write(&executable, b"test executable placeholder").unwrap();

    drop(Installation::begin(root, &executable).unwrap());
    assert_workflow_bundle(root);
    drop(Installation::begin(root, &executable).unwrap());
    assert_eq!(ProjectSupport::preview(root).unwrap().changes, 0);

    let report = uninstall(root).unwrap();
    assert_eq!(report["complete"], true, "{report}");
    assert!(!root.join(".agents/skills").exists());
    assert!(executable.is_file());
}

#[test]
fn sync_installs_workflow_bundle_and_preserves_locally_edited_support_files() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    {
        let support = ProjectSupport::lock(root).unwrap();
        let report = support.sync().unwrap();
        assert_eq!(report.errors, 0);
        assert_eq!(report.conflicts, 0);
    }
    assert_workflow_bundle(root);
    let edited = root.join(".agents/skills/codebase-design/DEEPENING.md");
    fs::write(&edited, b"local architecture guidance").unwrap();
    let support = ProjectSupport::lock(root).unwrap();
    let report = support.sync().unwrap();
    assert_eq!(report.conflicts, 1);
    assert_eq!(fs::read(&edited).unwrap(), b"local architecture guidance");
}

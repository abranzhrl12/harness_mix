use harness_skills::{
    installation::{Installation, uninstall},
    project_support::ProjectSupport,
};
use std::{fs, path::Path};

const REQUIRED_FILES: &[&str] = &[
    "harness-word/SKILL.md",
    "documentation/SKILL.md",
    "documentation/LICENSE",
    "harness-workflow/SKILL.md",
    "harness-workflow/SUPERPOWERS.md",
    "harness-workflow/references/operating-rules.md",
    "harness-read/references/search-and-results.md",
    "systematic-debugging/SKILL.md",
    "systematic-debugging/references/multi-component-investigation.md",
    "systematic-debugging/root-cause-tracing.md",
    "systematic-debugging/defense-in-depth.md",
    "systematic-debugging/condition-based-waiting.md",
    "systematic-debugging/condition-based-waiting-example.ts",
    "systematic-debugging/find-polluter.sh",
    "systematic-debugging/LICENSE",
    "test-driven-development/SKILL.md",
    "test-driven-development/references/red-green-example.md",
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

#[test]
fn bundled_word_guide_connects_evidence_diagrams_and_visual_review() {
    let guide = include_str!("../../../.agents/skills/harness-word/SKILL.md");
    for required in ["harness-read", "harness_word", "harness_word_preview", "harness_diagram", "image/png", "unknown"] {
        assert!(guide.contains(required), "missing workflow step: {required}");
    }
}

#[test]
fn bundled_workflow_routes_supported_mcp_tasks_and_respects_user_preferences() {
    let workflow = include_str!("../../../.agents/skills/harness-workflow/SKILL.md");
    for guide in ["harness-read", "harness-edit", "harness-browser", "harness-sql", "harness-xlsx", "harness-svg", "harness-word", "harness-setup"] {
        assert!(workflow.contains(guide), "missing MCP guide: {guide}");
    }
    let rules = include_str!("../../../.agents/skills/harness-workflow/references/operating-rules.md");
    assert!(rules.contains("Explicit user instructions take precedence"));
    assert!(rules.contains("reuse `data.session`"));
}

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

//! Embedded skill catalog shipped by this crate.

pub(crate) struct SkillSource {
    pub(crate) id: &'static str,
    pub(crate) relative_path: &'static str,
    pub(crate) content: &'static str,
}

macro_rules! skill_file {
    ($id:literal, $file:literal) => {
        SkillSource {
            id: $id,
            relative_path: concat!(".agents/skills/", $id, "/", $file),
            content: include_str!(concat!("../../../.agents/skills/", $id, "/", $file)),
        }
    };
}

pub(crate) const SKILLS: &[SkillSource] = &[
    SkillSource {
        id: "harness-read",
        relative_path: ".agents/skills/harness-read/SKILL.md",
        content: include_str!("../../../.agents/skills/harness-read/SKILL.md"),
    },
    skill_file!("harness-read", "references/search-and-results.md"),
    SkillSource {
        id: "harness-edit",
        relative_path: ".agents/skills/harness-edit/SKILL.md",
        content: include_str!("../../../.agents/skills/harness-edit/SKILL.md"),
    },
    SkillSource {
        id: "harness-browser",
        relative_path: ".agents/skills/harness-browser/SKILL.md",
        content: include_str!("../../../.agents/skills/harness-browser/SKILL.md"),
    },
    SkillSource {
        id: "harness-sql",
        relative_path: ".agents/skills/harness-sql/SKILL.md",
        content: include_str!("../../../.agents/skills/harness-sql/SKILL.md"),
    },
    SkillSource {
        id: "harness-xlsx",
        relative_path: ".agents/skills/harness-xlsx/SKILL.md",
        content: include_str!("../../../.agents/skills/harness-xlsx/SKILL.md"),
    },
    SkillSource {
        id: "harness-svg",
        relative_path: ".agents/skills/harness-svg/SKILL.md",
        content: include_str!("../../../.agents/skills/harness-svg/SKILL.md"),
    },
    SkillSource {
        id: "harness-setup",
        relative_path: ".agents/skills/harness-setup/SKILL.md",
        content: include_str!("../../../.agents/skills/harness-setup/SKILL.md"),
    },
    // Workflow skills and their upstream support files are embedded for offline init.
    skill_file!("harness-word", "SKILL.md"),
    skill_file!("harness-workflow", "SKILL.md"),
    skill_file!("harness-workflow", "SUPERPOWERS.md"),
    skill_file!("harness-workflow", "references/operating-rules.md"),
    skill_file!("systematic-debugging", "SKILL.md"),
    skill_file!("systematic-debugging", "LICENSE"),
    skill_file!(
        "systematic-debugging",
        "references/multi-component-investigation.md"
    ),
    skill_file!("systematic-debugging", "root-cause-tracing.md"),
    skill_file!("systematic-debugging", "defense-in-depth.md"),
    skill_file!("systematic-debugging", "condition-based-waiting.md"),
    skill_file!("systematic-debugging", "condition-based-waiting-example.ts"),
    skill_file!("systematic-debugging", "find-polluter.sh"),
    skill_file!("systematic-debugging", "CREATION-LOG.md"),
    skill_file!("systematic-debugging", "test-academic.md"),
    skill_file!("systematic-debugging", "test-pressure-1.md"),
    skill_file!("systematic-debugging", "test-pressure-2.md"),
    skill_file!("systematic-debugging", "test-pressure-3.md"),
    skill_file!("test-driven-development", "SKILL.md"),
    skill_file!("test-driven-development", "LICENSE"),
    skill_file!("test-driven-development", "references/red-green-example.md"),
    skill_file!("test-driven-development", "writing-good-tests.md"),
    skill_file!("verification-before-completion", "SKILL.md"),
    skill_file!("verification-before-completion", "LICENSE"),
    skill_file!("improve-codebase-architecture", "SKILL.md"),
    skill_file!("improve-codebase-architecture", "LICENSE"),
    skill_file!("improve-codebase-architecture", "HTML-REPORT.md"),
    skill_file!("improve-codebase-architecture", "agents/openai.yaml"),
    skill_file!("codebase-design", "SKILL.md"),
    skill_file!("codebase-design", "LICENSE"),
    skill_file!("codebase-design", "DEEPENING.md"),
    skill_file!("codebase-design", "DESIGN-IT-TWICE.md"),
    skill_file!("codebase-design", "agents/openai.yaml"),
    skill_file!("domain-modeling", "SKILL.md"),
    skill_file!("domain-modeling", "LICENSE"),
    skill_file!("domain-modeling", "ADR-FORMAT.md"),
    skill_file!("domain-modeling", "GLOSSARY-FORMAT.md"),
    skill_file!("domain-modeling", "agents/openai.yaml"),
    skill_file!("grilling", "SKILL.md"),
    skill_file!("grilling", "LICENSE"),
    skill_file!("grilling", "agents/openai.yaml"),
];

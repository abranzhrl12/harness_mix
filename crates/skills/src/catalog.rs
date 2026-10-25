//! Embedded skill catalog shipped by this crate.

pub(crate) struct SkillSource {
    pub(crate) id: &'static str,
    pub(crate) relative_path: &'static str,
    pub(crate) content: &'static str,
}

pub(crate) const SKILLS: &[SkillSource] = &[
    SkillSource {
        id: "harness-read",
        relative_path: ".agents/skills/harness-read/SKILL.md",
        content: include_str!("../../../.agents/skills/harness-read/SKILL.md"),
    },
    SkillSource {
        id: "harness-edit",
        relative_path: ".agents/skills/harness-edit/SKILL.md",
        content: include_str!("../../../.agents/skills/harness-edit/SKILL.md"),
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
];

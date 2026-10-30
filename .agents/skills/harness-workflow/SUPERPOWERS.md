# Local workflow integration

This bundle includes adapted local entrypoints for
`systematic-debugging`, `test-driven-development`, and
`verification-before-completion`. Their upstream MIT notices remain in each
skill directory. The concise entrypoints keep the methods; examples and
task-specific techniques are linked as references. This is not the full
Superpowers plugin.

`harness-workflow/SKILL.md` selects only the needed stage. Shared Harness rules
live in `references/operating-rules.md`; it is read once per task, not once per
tool call. The source of truth is `.agents/skills`; `crates/skills/src/catalog.rs`
embeds those files for offline setup.

Project `agent-harness init` installs skills and project/client configuration,
then reports the explicit next `index` step. It does not build the index.
`workspace init` prepares skills for projects already registered in its
manifest; `workspace add` prepares the added project. Client discovery/reload
remains independent from MCP reconnection.

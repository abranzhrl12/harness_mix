# harness-skills

Reusable skill synchronization for Harness projects. The crate embeds seven
Harness skills, a local workflow integration, and seven upstream workflow skills, including supporting documents and
licenses. It tracks every managed file with a SHA-256 manifest, preserves local
edits, and supports preview/check and opt-in startup synchronization.

The implementation is split into private modules: `catalog` owns embedded
sources, `sync` plans and applies synchronization, `manifest` tracks managed
hashes, `preferences` handles the opt-in setting, `storage` enforces safe file
access, `lock` serializes writes, and `report` defines the public result types.
The library facade exports synchronization, report/options types, project
support, and the reversible installation lifecycle. CLI `init` calls
`installation::Installation::begin`, which installs the same embedded catalog
as `skills sync`.

The CLI owns project initialization and its database/MCP example files. This
crate only provides the skill catalog, safe file operations, synchronization,
and auto-sync preferences. It depends on `harness-domain` for hashing and the
tool contract version; it has no CLI, MCP, application, or async-runtime
dependency.

## Bundled workflow skills

`harness-workflow` routes a task to the required stage without loading every
guide. Harness task guides share
`.agents/skills/harness-workflow/references/operating-rules.md` for session,
authorization, failure, index, and evidence rules. The debugging and TDD
entrypoints are local concise adaptations; their upstream-derived methods remain
available through linked references. `harness-read` and `harness-edit` link to
the needed stages directly rather than back to the workflow router. MIT licenses
and notices are retained. This is not an installation of the full Superpowers
plugin.

| Source | Skills |
| --- | --- |
| [obra/superpowers](https://github.com/obra/superpowers) | systematic-debugging, test-driven-development, verification-before-completion |
| [mattpocock/skills](https://github.com/mattpocock/skills) | improve-codebase-architecture, codebase-design, domain-modeling, grilling |

The latter four form the architecture workflow: architecture review links to
the codebase design, domain modeling, and grilling guides. The debugging route
links to test-driven development and verification before completion. References,
formats, agent metadata, and MIT licenses needed by those skills are embedded
alongside each SKILL.md, so project initialization needs no network download.
Upstream scripts and examples are copied as data; initialization never runs them.

Each upstream skill directory contains its LICENSE with the original notice.
The vendored source files live in the repository's .agents/skills directory;
catalog.rs explicitly lists the files included in the executable. To update a
skill, review its upstream changes and references, retain its license, update
the catalog when files change, and run cargo test -p harness-skills.

Installing a skill makes it available to a compatible client; it does not force
the agent to load every skill or run a workflow. Architecture review declares
disable-model-invocation: true upstream and should be explicitly requested.
Client discovery and reload behavior remain client-specific.

Workflow files use the existing per-file ownership rules. Repeating init is
idempotent; skills sync preserves local changes and reports conflicts; uninstall
removes only unchanged owned files and directories. Files already present before
installation are retained or restored according to the installation receipt.

The new catalog takes effect in executables compiled from this source. An older
installed executable continues to distribute its own embedded catalog until it
is updated. This integration does not replace or restart an installed server.

---
name: harness-setup
description: Install or remove Harness in a project, register MCP clients, or configure a multi-project workspace. Not for routine reads or index maintenance.
---

# Set Up Harness

Read [shared operating rules](../harness-workflow/references/operating-rules.md)
once for setup work. Setup writes project or client files; do it only when the
user requests installation or configuration.

## Install in one project

Check the existing `agent-harness --version` executable first. If the user
requests project installation, run `agent-harness init --root <project>` from
the project's intended root.

`init` installs bundled skills, project configuration files, a receipt, and
local MCP client entries. It prepares the index configuration, but does not
create or populate `.codegraph`; the user must separately authorize and run the
`index` command. It preserves pre-existing or locally edited files and reports
conflicts. Re-running `init` resumes an incomplete installation. It does not
replace the shared executable. `uninstall` removes only unchanged files recorded
in the receipt and preserves files it cannot safely claim.

Do not change client credentials, global configuration, permissions, or other
MCP entries. If the user asks to connect a client manually, identify its actual
configuration file, preserve unrelated entries, and use the executable with
`args: ["--root", "ABSOLUTE_PROJECT_PATH"]`.

## Multi-project workspace

- `workspace init` creates the manifest, example, and guide when missing. It
  prepares skills for projects already registered in that manifest; an empty
  manifest has no projects to prepare, so the workspace directory itself does
  not receive project skills.
- `workspace add <alias> <path>` registers the project and prepares its skills.
  When a running workspace server discovers newly registered projects, it also
  prepares those projects. Existing `auto_sync: false` preferences are honored;
  an absent preference is created enabled. Skill conflicts preserve local
  files and are reported.
- `workspace check` verifies registered roots and reports index presence; it
  does not establish index compatibility or freshness. The example manifest is
  documentation, not an active project.
- For skills only, use `agent-harness --root <project> skills sync`; do not run
  project `init` as a shortcut.

Use `--workspace-config <absolute-manifest>` for a multi-project server entry.
No arguments can discover the nearest manifest from the process working
directory, but an explicit path avoids that dependency. A client may need a
reload after changing its MCP entry or installed skills. Client skill discovery
and MCP reconnection are separate steps.

After connecting, bootstrap once to verify the selected root or registered
project list, then reuse its session. Bootstrap opens or restores a session; it
does not initialize or refresh an index.

## Indexing relationships

Reads and searches do not require an index. `harness_explore` relationship
queries need a compatible index. Create, synchronize, or rebuild it only when
the user authorizes index maintenance. If `.codegraph/` already exists, check
compatibility before using supported maintenance; never delete it to repair it.

---
name: harness-setup
description: Install or uninstall Harness in a project, connect MCP, and prepare initial usage. Not for routine code reading or routine index maintenance.
---

# Set Up Harness

Configure the project and client once; do not repeat setup before every read.
Reuse an active connection and valid session.

## Executable

Check the known installation with `agent-harness --version`. The version output
is for diagnostics; do not repeat it in every response. If the command is not
on `PATH`, locate the existing binary before installing.

If asked to install on Windows x64, run:
`npm install --global @abranzhrl1212/agent-harness@<version>` using the version
requested by the user or verified in the current registry. Never download
unknown binaries.

## Project Configuration

Only when asked to install Harness into a project, run `agent-harness init` from
the workspace root, or `agent-harness init --root <absolute-path>`. This
installs the bundled skills, configures local MCP entries for Codex and
Antigravity, and prepares the index. If an installation already exists, it
synchronizes without deleting existing data. Re-running `init` resumes an
incomplete installation without extra flags. Preserve unrelated MCP servers,
permissions, credentials, and locally edited files.

`agent-harness uninstall` removes only the artifacts recorded in
`.agents/harness-installation.json`, restores prior file versions, and deletes
only the index created by that installation. It preserves pre-existing indexes
and user-modified files, reporting any conflicts, and never deletes the shared
executable. If the index is locked by an active process, close the project's MCP
connections before retrying.

To connect a client manually, identify the client and its actual config file.
Use the command and arguments registered by `init` while preserving other
servers, credentials, and permissions. The stdio entry uses the executable with
`args: ["--root", "ABSOLUTE_PROJECT_PATH"]`. Never launch a stdio server in an
interactive terminal as a substitute for registering it in the client config.

## First Connection and Session

For several projects, use `agent-harness workspace init` in their common folder.
It creates `harness-workspace.json`, `harness-workspace.example.json` and
`HARNESS-WORKSPACE.md`, preserving existing files. Register real roots with
`workspace add <alias> <path>` or edit `projects`, then run `workspace check`.
Relative roots are resolved from the manifest's directory. The example is not
active configuration and an empty manifest cannot serve projects.

Workspace initialization does not install project skills, configure MCP clients
or create indexes. If only skills are requested, use
`agent-harness --root <project> skills sync` for each authorized project instead
of project `init`. The latter also registers single-project MCP entries and
prepares the index; do not run it as a shortcut for multiproject configuration.

For the multiproject MCP entry, use `args: ["--workspace-config",
"ABSOLUTE_MANIFEST_PATH"]`. Alternatively, no arguments discover the nearest
`harness-workspace.json` from the process working directory towards its parents.
This works only when the client launches from the intended workspace. Explicit
`--root` always selects a single project. Manifest changes reload between MCP
tool calls, checked at most every 250 ms, without restarting. Invalid changes
preserve the last valid configuration; inspect bootstrap's `workspaceReload`
status and stderr. Refresh bootstrap with the existing session to confirm roots.

After connecting, call `harness_bootstrap`: verify `root` and reuse
`data.session` in subsequent tool calls. In the current implementation,
bootstrap opens or restores a session; it does not initialize the code index. Do
not call bootstrap before every read—only when starting a new session or
restoring a known one via `session`. Never claim the MCP connection is
operational without a successful `harness_bootstrap` call. If client reload is
required to pick up new MCP config or skills, inform the user of that pending
step.

In workspace mode, verify the registered project list and each root rather than
requiring a single top-level `root`. Keep aliases with source evidence and use
`alias:/path` to route operations. Multiproject startup and registration prepare skills for registered projects. An absent preference is created enabled; an existing `auto_sync: false` is respected. Conflicts preserve local edits and are reported. A client may need to refresh skill discovery independently of reconnecting MCP.

## Initial Index (Relationships Only)

Reading and searching code do not require an index. If relationship/graph
queries (`harness_explore`) are needed and the user authorizes creating an index
in an unindexed project, run:
1. `agent-harness --root <absolute-path> --init-index`
2. `agent-harness --root <absolute-path> --reindex`

If `.codegraph/` already exists, check compatibility before running supported
maintenance; never delete `.codegraph/` or repeat `--init-index`. `indexed=true`
only indicates that `.codegraph/` exists, not that the index is compatible,
complete, or fresh. Initial setup does not guarantee freshness after later code
edits; reindexing requires authorization.

## Subsequent Usage

Skills define the workflow; tools return operational data and diagnostics.
`harness_bootstrap` does not load `AGENTS.md` or legacy rule files. For code
tasks, load `harness-read` or `harness-edit` according to the task.

Shell commands are used here strictly for installation and configuration—never
to bypass Harness tool failures. Do not initialize or rebuild `.codegraph/`
routinely: an index is not required to read or search code.

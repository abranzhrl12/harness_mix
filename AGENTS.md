# Harness MCP

Use Harness MCP as the primary channel to investigate, read, edit, and verify
this project. Discover its live tool names, run `harness_bootstrap`, verify the
workspace root, and reuse `data.session`. Follow the live tool schemas rather
than assuming older versions.

For the shared root/session, authorization, index, failure, and evidence rules,
read [operating rules](.agents/skills/harness-workflow/references/operating-rules.md) once per
task. Tool guides link to that source instead of repeating it.

## Load Only the Needed Guide

| Task | Local Skill |
| --- | --- |
| Coordinate a feature or bug fix with debugging, TDD and evidence | `.agents/skills/harness-workflow/SKILL.md` |
| Search, read code, or trace relationships across layers | `.agents/skills/harness-read/SKILL.md` |
| Replace, write, move, and verify code | `.agents/skills/harness-edit/SKILL.md` |
| Diagnose web apps via console, network, DOM, and screenshots | `.agents/skills/harness-browser/SKILL.md` |
| Query SQL Server or PostgreSQL | `.agents/skills/harness-sql/SKILL.md` |
| Read an existing XLSX workbook | `.agents/skills/harness-xlsx/SKILL.md` |
| Inspect or preview an SVG file | `.agents/skills/harness-svg/SKILL.md` |
| Install, initialize, or verify the Harness CLI/server | `.agents/skills/harness-setup/SKILL.md` |

Read the applicable skill before acting; combine skills only when the task
requires it. Read known symbols directly with `harness_symbols` without a prior
search, outline, or signature call. Group independent reads in `harness_batch`
(`{ "jobs": [...] }`).

## Project-specific constraints

- Respect protected paths, external scopes, active profiles, and concurrent
  changes. A project alias is not permission to cross into another root.
- If a graph query is authorized in an unindexed project, create then reindex
  with `--root <project> --init-index` followed by `--root <project> --reindex`.
  If an index already exists, inspect compatibility and use supported
  maintenance; never delete it to repair it.
- Syntax validation is not a full build or runtime check. Keep `validated` and
  `pending` evidence distinct. Checkpoints are not snapshots. Never commit or
  push unless explicitly requested.

## If the Connection Is Missing

The known executable is the global npm binary from the current installation:
`C:/Users/AbrahamRodriguez/AppData/Roaming/npm/node_modules/@abranzhrl1212/agent-harness/bin/agent-harness.exe`
or the local repository release build (`<root>/target/release/agent-harness.exe`).
Verify existence and `--version`; never download unknown binaries. The MCP entry
uses that command with `args: ["--root", "ABSOLUTE_PROJECT_PATH"]`.

Configure client files only if requested by the user: identify the client and
its actual config file, preserve other servers, and never alter credentials or
permissions. If the binary is missing, locate a known installation or build from
source with `cargo build --release --locked` when configuration is part of the
task. Never start a stdio server in a terminal as a substitute for registering
it in the client. If a client reload/reconnection is needed, report that pending
step; never claim an operational connection without a real `harness_bootstrap`
call.

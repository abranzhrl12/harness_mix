# Harness MCP

Use Harness MCP as the primary channel to investigate, read, edit, and verify
this project. Discover its live tool names, run `harness_bootstrap`, verify the
workspace root, and reuse `data.session`. Follow the live tool schemas rather
than assuming older versions.

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

Read the applicable skill in full before acting; combine skills only when the
task requires it. Read known symbols directly with `harness_symbols` without a
prior search, outline, or signature call. Group multiple independent reads into
a single `harness_batch` call (`{ "jobs": [...] }`).

## Invariants

- Preserve required source context, documentation, hashes, and notices regarding
  omissions, truncation, and freshness. A summary does not replace full evidence.
- Respect concurrent changes, protected paths, external scopes, and active
  profiles. Permission to investigate does not authorize editing; having tools
  available does not expand permissions.
- Do not initialize, synchronize, or rebuild the code index routinely. Reading
  and searching do not require an index; relationship queries (`harness_explore`)
  require a compatible one. Indexing writes to `.codegraph/` and requires user
  authorization. Never delete indexes to "repair" them. For an unindexed project
  with user authorization, run `--root <project> --init-index` first and then
  `--root <project> --reindex`. If an index already exists, inspect
  compatibility and use supported maintenance commands.
- On transport, timeout, or runtime failure, stop equivalent retries, diagnose
  with `harness_doctor`, and report the blocker. Never bypass failures with
  direct filesystem access or shell commands. An unsupported capability is
  distinct from a failure: explain the limitation before using an authorized
  alternative.
- Valid syntax does not prove full compilation or runtime correctness. Report
  applied edits, `validated` checks, and `pending` checks. Checkpoints are not
  snapshots. Never commit or push unless explicitly requested.

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

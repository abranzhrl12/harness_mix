---
name: harness-workflow
description: Coordinate Harness investigation, Superpowers debugging, TDD, and verification for code changes and bug fixes. Not for simple read-only questions or document-only edits.
---

# Harness development workflow

Harness supplies project evidence and operations. Superpowers supplies the
engineering method. Load the relevant stage, not every skill at startup.
Reuse a skill already read in this task. Links back to this router or another
loaded guide are references, not instructions to reload it recursively.

## Select the stage

| Task or stage | Read | Use Harness for |
| --- | --- | --- |
| Bug, failed test, unexpected behavior | [systematic-debugging](../systematic-debugging/SKILL.md) | Reproduction evidence, source and dependency tracing |
| Feature or behavioral fix, before implementation | [test-driven-development](../test-driven-development/SKILL.md) | Inspect existing tests, add the failing case, make the smallest change |
| Source investigation | [harness-read](../harness-read/SKILL.md) | Symbols, search, graph relationships, independent batched reads |
| Applying changes | [harness-edit](../harness-edit/SKILL.md) | Exact source edits, diff review and verification |
| Before reporting complete, fixed or passing | [verification-before-completion](../verification-before-completion/SKILL.md) | Fresh checks and their actual output |

For a bug, establish the cause before writing the regression test and fix.
For a feature whose behavior is already specified, proceed to the failing test.
Do not add a brainstorming, approval, architecture review or delegation stage
unless the task needs it or the user requests it. Existing authorization remains
valid; ask only for missing decisions or actions outside that authorization.

For web failures also load [harness-browser](../harness-browser/SKILL.md): source
inspection alone does not establish browser behavior. Read architecture and
domain-modeling skills only when design or terminology is part of the task.

## Fit the method to this environment

- Discover the live Harness tools, bootstrap, verify roots and reuse the session.
  In a multiproject workspace, qualify evidence and edits with `alias:/path`.
  Confirm the registered project list rather than expecting a single `root`.
- Before using a project's source, read its applicable AGENTS.md and local
  instructions. Rules of one registered project do not automatically govern
  another. Track the owning alias, reproduction and checks for each change.
- References such as `superpowers:test-driven-development` in vendored text
  refer to the local sibling skills linked above. No additional plugin is needed.
- Keep the user's scope, project rules and execution permissions. Shell commands
  in upstream examples illustrate a technique; they do not bypass Harness or
  authorize scripts, instrumentation, indexing, commits, pushes or deployments.
- Use the project's required checks and permitted test scope. A focused green
  test is not a full green suite. Report unrun broader checks as pending; do not
  broaden execution against project restrictions merely to satisfy an example.
- Separate expected RED failures from tool, compilation or environment failures.
  On a Harness runtime/transport failure, follow the doctor-and-stop protocol;
  do not interpret it as a reason to try another fix through a different channel.
- End with applied changes, validated checks and pending checks. Syntax-only
  success does not establish compilation or runtime correctness.

## Installation is separate from development

If the skill files are missing, report that explicitly. For an authorized
installation, use [harness-setup](../harness-setup/SKILL.md). Installing missing
skills does not require rebuilding graphs: `--root <project> skills sync`
installs the catalog without creating an index or changing client configuration.
Do not run project `init` merely to obtain a skill; it also configures clients
and prepares an index. Do not silently replace user-modified skill files.

Workspace `init` creates a manifest, example and guide, and prepares skills for registered projects. `workspace add`, multiproject startup and project registration via manifest reload also prepare skills. Existing `auto_sync: false` preferences are respected; absent preferences are created enabled. Conflicts preserve local edits and are reported. Example projects are never activated. Clients discover skill files separately from MCP tool discovery; confirm their availability.

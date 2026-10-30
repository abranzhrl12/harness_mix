---
name: harness-workflow
description: Route Harness-backed code changes through the needed investigation, TDD, and verification steps. Not for simple reads or document-only edits.
---

# Harness workflow

Read [shared operating rules](references/operating-rules.md) once per task that
uses Harness. Load only the stages the task needs; keep the router available
without rereading it when its links are followed.

| Situation | Read |
| --- | --- |
| Bug, failed test, unexpected behavior, or performance issue | [systematic-debugging](../systematic-debugging/SKILL.md) before proposing a fix |
| New behavior or bug fix, before implementation | [test-driven-development](../test-driven-development/SKILL.md) |
| Read or trace source | [harness-read](../harness-read/SKILL.md) |
| Apply a code change | [harness-edit](../harness-edit/SKILL.md) |
| Browser behavior, console, or hydration | [harness-browser](../harness-browser/SKILL.md) |
| Claim work complete, fixed, or passing | [verification-before-completion](../verification-before-completion/SKILL.md) |

For a bug fix, investigate and establish the cause before writing the regression
test; then observe the expected failure, make the smallest change, and verify
the permitted scope. For specified new behavior, start with TDD. A read-only
question or document-only edit does not need a synthetic runtime test.

Respect the user's authorization and each project's `AGENTS.md` and test scope.
Do not add approval, architecture-review, or delegation stages unless the task
needs them. For setup or removal, use [harness-setup](../harness-setup/SKILL.md);
do not run setup as a shortcut for routine reads or index maintenance.

When subagents are available and delegation is authorized, use them only when
the work benefits from it. For independent investigations that can run without
shared state, consult `dispatching-parallel-agents` if installed. For an
existing implementation plan with independently delegable tasks, consult
`subagent-driven-development` if installed. Keep tightly coupled or small work
local. Give each worker a clear scope, coordinate shared files and tests, and
verify the combined result. Host instructions and user constraints take priority
over either skill; their absence must not block the task.

The local integration of the imported debugging and TDD methods is recorded in
[SUPERPOWERS.md](SUPERPOWERS.md). That file is background, not another required
skill to load for every change.

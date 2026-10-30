---
name: harness-edit
description: Edit code and verify changes with Harness MCP. Not for read-only investigation.
---

# Edit and verify

Read [shared operating rules](../harness-workflow/references/operating-rules.md)
once per task. For a bug, establish its cause with
[systematic-debugging](../systematic-debugging/SKILL.md); before a behavior
change, use [test-driven-development](../test-driven-development/SKILL.md).

Reuse the verified project/session and preserve unrelated work. Use advertised,
authorized tools with current source context:

- `harness_replace`: use exact source text and non-overlapping chunks; search
  results with enough context can go directly to replace. On `STALE`, fetch the
  new source and adapt, never force an old version.
- `harness_write`: distinguish creation from replacement and preserve any
  required `expectedHash` precondition.
- `harness_move`: does not rewrite imports or overwrite destinations. If the
  move succeeded but later index maintenance failed, do not repeat the move.

Verify with the tool and scope appropriate to the change. For small JS,
TypeScript, CSS, Razor, or C# edits, `harness_verify` can run a fast per-file
syntax/structural check; it does not establish a full build. For long checks,
use `harness_jobs` only when authorized. For browser behavior, also follow
[harness-browser](../harness-browser/SKILL.md) and reproduce with active capture.

Before claiming success, read fresh output and follow
[verification-before-completion](../verification-before-completion/SKILL.md).
Report `validated` and `pending` checks separately; do not commit, push, replace
installed binaries, or use a shell workaround for a rejected Harness operation.

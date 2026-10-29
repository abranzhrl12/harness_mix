---
name: harness-edit
description: Edit code and verify changes with Harness MCP. Not for read-only investigation.
---

# Edit and Verify

For a feature or behavioral fix, read
[harness-workflow](../harness-workflow/SKILL.md) to select the local Superpowers
stages: cause investigation for bugs, TDD before implementation, and fresh
verification before completion. Load only the applicable stage; documentation
and configuration-only edits do not need an artificial runtime test.

Reuse a session whose workspace root was verified via `harness_bootstrap`. Check
Git status and preserve unrelated changes. Fetch any missing exact source via
Harness; do not re-read valid context or load another skill if you already have
the required evidence.

## Apply the Change

- `harness_replace`: pass `path`, exact `oldText`, and `newText`. `hash` is
  optional; when omitted, the server uses its internal file read. Never perform
  an extra read just to obtain a hash. Disambiguate with `line` when needed;
  group edits to the same file into unique, non-overlapping `chunks`.
- If you locate the target with `harness_search` (`kind=text`), pass
  `contextLines: 5` (or `3..10`) in the search call to receive the surrounding
  lines and line numbers (`N | `) directly, then call `harness_replace`
  immediately without an intermediate `harness_read`.
- Never use signatures, summaries, or line-number prefixes (`N | `) as `oldText`.
  On `STALE`, fetch the affected context and adapt your edit; do not force the
  stale version.
- `harness_write`: distinguish file creation from replacement. Preserve
  `expectedHash` when required as an overwrite precondition.
- `harness_move` does not rewrite imports or overwrite existing destinations. If
  the file move succeeded and only subsequent index maintenance failed, do not
  repeat the move.
- Use advertised and authorized tools. If higher-priority instructions require
  `apply_patch`, use it with current context obtained through Harness; never
  bypass a permission rejection.

## Verify and Deliver

Review the diff with `harness_git` and run verification proportional to the
change with `harness_verify`; use background `harness_jobs` when long-running
follow-up is needed. Check the tool schema before requesting filters or profiles.

### Fast Per-File Verification vs. Full Compilation

For small or localized JS, Razor (`.cshtml`), or C# (`.cs`) edits, pass the
modified file paths to `harness_verify` for fast per-file verification. This
runs lightweight syntax and structural checks on the changed files and
explicitly reports `validated` checks versus `pending` checks (such as full
solution compilation or integration tests) without waiting for a full project
build. Valid syntax alone does not prove full compilation or runtime
correctness; always ensure verification evidence matches your latest edit and
clearly report both `validated` and `pending` checks.

### Browser and Runtime Verification

When fixing web UI, interaction, or hydration bugs, also follow
`../harness-browser/SKILL.md`: reproduce with active capture and check console,
network, and DOM behavior after the change. If no browser is connected or you
cannot reproduce the flow, report runtime verification as `pending`, even if the
code compiles.

If a capability is unsupported, explain the limitation before using an
authorized alternative. On transport, timeout, or runtime failure, diagnose with
`harness_doctor`; never bypass failures with shell commands, broader permissions,
or index rebuilds.

Deliver the change along with executed (`validated`) and `pending`
verifications. Do not commit, push, or replace installed binaries unless
explicitly asked.

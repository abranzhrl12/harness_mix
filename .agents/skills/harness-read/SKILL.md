---
name: harness-read
description: Investigate code, read symbols, and query dependencies with Harness MCP. Not for SQL, XLSX, or SVG.
---

# Read and Investigate

Use the tool names and schemas advertised by the connection; never invent parameters.
This guide covers reading and investigation, not installation or initial setup.
If the issue occurs in a web application (visual, interaction, console, or
hydration), also load `../harness-browser/SKILL.md` to gather browser evidence;
reading code alone does not verify what happens in the tab.

## Choose by the Missing Information

Prioritize `harness_explore` to investigate how code works and follow its
relationships; avoid chains of text searches and file-by-file reads. To locate
literal text or an unknown path, use `harness_search`; to read a known symbol
when relationships are not needed, use `harness_symbols` directly.

| Need | Tool |
| --- | --- |
| Find a file or literal text | `harness_search`, `kind=files` or `text` (`kind=files` matches filename substrings, not globs: `*.cs` matches nothing; use `Almacen` or filter with `globs`). If you plan to edit matches, include `contextLines: 5` (or `3..10`) to receive the numbered block ready for `harness_replace` without an intermediate `harness_read` |
| Read a known implementation | `harness_symbols`, `action=read`, `path`, `symbol` |
| Choose a symbol from a file | `harness_symbols`, `action=outline`, `path` |
| Get only the signature | `harness_symbols`, `action=read`, `view=signature` |
| Understand calls, dependencies, or impact | `harness_explore`, real selector and appropriate view |
| Trace a field or identifier across layers | `harness_explore`, `query` or `symbol` for cross-layer data tracing (`form field → JSON/JS → C# model → SP parameter → SQL column`) |
| Read non-symbol text or unsupported formats | `harness_read`, sufficient line range |
| Retrieve omitted content | `harness_detail`, received reference (`detail.ref`) |
| Project fields from a large result | `harness_data_query`, JMESPath |

Read a known symbol directly without a prior search, outline, or signature call:

```json
{"action":"read","path":"src/module.rs","symbol":"validate"}
```

Symbol reads return exact source by default; they do not need `view=full` or
compaction flags. Line ranges are 1-based and inclusive. If ambiguous, select a
returned `container` or `symbolId`; do not guess them. A signature is not source
for editing. Preserve any hashes required by the tool, such as `expectedHash`
for Vue extraction; do not fetch a hash just to use `replace`, where it is optional.

### Block Reads

`harness_read` uses `path`, `start`, and `end`: 1-based inclusive line numbers,
up to 420 lines per call (`end - start + 1 <= 420`). If the range exceeds the
16 KB budget, it automatically truncates on complete line boundaries and returns
`nextStart` without failing with a `SIZE` error. Request only the range you need,
not 420 by default. For example: `{"path":"src/module.rs","start":80,"end":160}`.
If you need more, continue from `nextStart` (or the last `end + 1`) without
re-reading blocks or scanning the entire file when a symbol read or `explore`
answers the question.

## Relationships, Cross-Layer Tracing, and Limits

For calls, dependencies, or impact, start with `harness_explore`, not text
reference searches or file-by-file reads. When the file and symbol are known,
call it directly without prior search or outline calls:

```json
{"filePath":"src/module.rs","symbol":"validate","view":"summary"}
```

File selectors accept native path separators (`src\\module.rs` on Windows) and
relative `./` prefixes; the server normalizes paths before querying the index
and retries case-insensitively before reporting `TARGET_NOT_RESOLVED`.

### Cross-Layer Data Tracing

When investigating how a field or identifier (such as `tObservacionEmpleado`)
flows across layers, use `harness_explore` to trace the full chain end-to-end:
`form field → JSON/JS → C# model → SP parameter → SQL column`. Pass the
identifier in `query` (or `symbol` when anchored to a file) so `harness_explore`
links UI/Razor inputs, JavaScript payloads, C# DTO/model properties, stored
procedure parameters, and database columns in a single exploration pass instead
of manual multi-step searches.

### Views and Lean Output

`summary` returns relationships and signatures without bodies by design; do not
fetch bodies unless they change the answer. `full` (the default when `view` is
omitted) includes bodies: its text arrives in a lean block whose numbered code
(`N | `) uses a language fence matching the file extension (` ```csharp `,
` ```javascript `, etc.) in natural reading order (header, code, relationships
at the end); a compact meta-line carries status, freshness, coverage, and
`detail.ref` — never parse JSON from that text: the full canonical envelope is
in `structuredContent`, and `detail.ref` retrieves the complete untruncated
result via `harness_detail` without re-running the analysis. `impact` queries
inbound callers. If the location is unknown, use `search` to find a candidate;
if homonyms exist, narrow with `filePath` and the returned selectors. Literal
search remains appropriate for plain text. Add budgets only when needed and
supported by the view. `maxFiles` limits target files and loaded related sources,
not the node count.

The same lean rendering applies across read-only tools:
`harness_read` renders the snippet in a language fence with absolute `N | `
line numbers (the header shows the starting line); `harness_symbols`
`action=read` reuses that format and `action=outline` returns a
`Line | Kind | Name | Signature` table; `harness_sql` `action=query`/`describe`
deliver Markdown tables (always readonly for QUERY/SCHEMA) and routine
definitions arrive in a `sql` fence; `harness_git` `action=diff` wraps patches
in a `diff` fence and `action=status` groups tracked/conflicts/untracked files;
`harness_search` `kind=text` groups matches by file with numbered lines. In all
cases, the canonical envelope (full ranges, hashes, raw rows) remains in
`structuredContent`. Errors keep structured JSON with intact diagnostics.

Check coverage, freshness, and omissions before concluding. A source hash or
`syntaxOnly=true` does not certify the graph; a bounded empty result does not
prove absence. Retrieve only bodies or details that could change the answer.
If the graph is unavailable or stale, proceed with source reads and state that
limitation. Do not sync or reindex routinely: reading and searching do not
require an index; relationship queries require a compatible index, and index
maintenance requires user authorization.

## Batching Independent Reads

Reuse valid evidence. Group independent read/search/symbol/explore calls into a
single `harness_batch` with `{ "jobs": [...] }`; for `explore` jobs, pass
`syncIndex=false` when supported by the schema.

`harness_batch` returns inline per-job summaries (matches, status, and
diagnostics) by default first so you can inspect results immediately from a
single call. Each job also provides a `detail.ref` handle separately when you
need to retrieve the full untruncated output via `harness_detail` without
re-running the batch jobs. Do not shrink budgets so aggressively that you force
predictable follow-up fetches.

On transport, timeout, or runtime failure, stop equivalent retries, diagnose
with `harness_doctor`, and report the blocker; do not bypass it with shell
commands. An unsupported capability is different: explain the limitation before
using an authorized alternative. Stop once the evidence answers the question and
state any limits that affect the conclusion.

---
name: harness-read
description: Search, read code, or trace relationships across layers with Harness MCP. Not for SQL, XLSX, or SVG.
---

# Read and Investigate

Use the advertised tool names and live schemas. Read
[shared operating rules](../harness-workflow/references/operating-rules.md) once
per task that uses Harness.

## Choose the next read

| Need | Tool |
| --- | --- |
| Locate a file or literal text | `harness_search` (`kind=files` or `text`) |
| Read a known symbol | `harness_symbols`, `action=read`, with `path` and exact `symbol` |
| Choose symbols in a known file | `harness_symbols`, `action=outline` |
| Follow calls, dependencies, or impact | `harness_explore` with an exact file/symbol/query |
| Read unsupported or non-symbol text | `harness_read` with a bounded line range |
| Recover omitted output | `harness_detail` with the returned `detail.ref` |
| Project fields from a large result | `harness_data_query` with JMESPath |

Read a known symbol directly; do not search or outline first. Use `summary` when
signatures/relationships answer the question, `full` for bodies, and `impact`
for inbound callers. If a relation is ambiguous, inspect returned selectors;
do not guess. `harness_explore` needs a compatible index, but reading and
searching source do not.

## Read and preserve evidence

`harness_read` line ranges are 1-based and inclusive, up to 420 lines. Default
is 120 lines; large ranges can truncate at the byte budget and return
`nextStart`. Keep source hashes, freshness, coverage, truncation, omissions,
diagnostics, and `detail.ref` when they affect the answer. A bounded empty result
does not prove absence.

In the default presentation, `structuredContent` carries the canonical result
envelope; `content` is a concise display and may omit source or metadata. This
applies to default bootstrap, read, symbols, search, and batch responses.
`HARNESS_MCP_PRESENTATION=legacy` selects older per-tool text presentation;
prefer `structuredContent` whenever supplied. For `harness_explore`, use
`dataTrace` as the canonical cross-layer trace field; legacy responses may also
include the compatibility duplicate `crossLayerTrace`.

For detailed search parameters, pagination, freshness fields, and cross-layer
trace handling, read [search and result details](references/search-and-results.md)
only when the question needs them. For browser console/DOM/hydration failures,
also use [harness-browser](../harness-browser/SKILL.md); source reading alone
does not confirm browser behavior. For bugs, use
[systematic-debugging](../systematic-debugging/SKILL.md) before suggesting a
fix; for implementation routing, follow the project's `AGENTS.md`.

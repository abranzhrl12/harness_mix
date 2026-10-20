# Planning and job output tools (Rust)

MCP responses use the shared Harness envelope: truncation is reported in
`omitted.truncated`; tool fields are in `data` and saved content is in `detail`.

`harness_explore {"view":"impact","filePath":"src/auth.rs","symbol":"validate_token","maxDepth":1,"limit":20}`
returns unique inbound callers, affected file counts and declaration locations.
These locations are not exact call sites. It reuses the bounded CodeGraph traversal
without synchronizing the index. Inspect `ambiguous`, candidates, file freshness,
traversal limits and unresolved entries before editing. Counts concern the returned
subgraph; dynamic references and macros may be missing. Detail retains the graph
and omitted callers. `requireMatchingFiles:true` rejects stale contributing files,
but does not certify global index coverage.

`harness_replace` with `preview:true` accepts `path`,
`hash`, either `oldText`/`newText` or `chunks`, and optional `session`. It prepares
the candidate with the same hash checks and validates syntax, returning `patch`,
`baseHash`, `candidateHash`, `syntax` and `published:false`. Syntax failures still
return the candidate diff. The unified diff includes three context lines around a
each hunk; alignment work is bounded and may produce a nonminimal diff for large
replacements. CRLF and missing final newlines are preserved. The compact
patch is limited to 12000 characters; retrieve the full patch with `harness_detail`
if truncated. To apply, call `harness_replace` with the same replacement and the
returned `baseHash` as the input `hash` (canonical) or `baseHash` (compatible alias),
never both; the source is checked again. Preview does not reserve files,
validate types or compile the project. Both planning tools are available in the
investigate profile as well as edit/full.

`harness_observe {"taskId":"...","filter":"ERROR|panic","lines":20,"before":2,"after":4}`
returns the newest matching lines plus context from each stream. `stream` accepts
`stdout`, `stderr` or `both`; omitted filters match all retained lines. `job_id` is
an alias for `taskId`. Follow subsequent output with the returned `cursor`.
Context is limited to the requested cursor window. Pipe fragments are joined
before regex matching; partial trailing lines and lossy decoding are reported.
`maxChars` caps UTF-8 text bytes (default 6000, maximum 16000); at most 400 lines
including context are returned, with truncation reported. Inspect `gap`,
`oldestCursor`, `outputLimited`, `truncated` and `persistenceError`; output evicted
from the bounded event journal cannot be recovered. No match is not proof of no
error. The tool is available in full profile alongside `harness_jobs`; it neither
starts arbitrary servers nor reads user-supplied log paths.

# Search and result details

Load this reference only when search, pagination, graph tracing, or a large
result affects the question.

## Search

- `kind=files` matches filename substrings, not globs (`*.cs` matches nothing);
  use a distinctive name and `globs` when supported.
- Text search is literal by default. When a multiword unpunctuated query has no
  literal match, the search tool may retrieve terms and rank them with BM25.
  `regex=true` uses line-based Rust regex; `caseSensitive=false` ignores case.
  `onlyMatching=true` returns deduplicated matches; with a regex capture group,
  group 1 is returned.
- AST patterns use the advertised `ast` language option. Do not combine AST
  mode with regex or case-sensitivity options.
- `contextLines=0..20` returns exact source context and a SHA-256 file hash.
  Treat those ranges as 1-based and inclusive. For `harness_replace`, use the
  returned `file`, hash, and complete context as `oldText` when they contain
  enough of the intended replacement; an omitted or truncated context is not
  safe replacement text.

## Pagination, coverage, and details

Follow `nextCall` with its cursor and offset to continue a captured result set.
Inspect `scanComplete`, `hasMore`, `snapshotOnly`, coverage, warnings, and
`omitted` before drawing conclusions. Pagination can continue only through the
captured results; a partial scan does not prove that files or matches are absent.
Use `harness_detail` with the returned `detail.ref` for omitted content without
rerunning an expensive operation.

For `harness_explore`, select `summary` for relationships and signatures,
`full` for bodies, or `impact` for inbound callers. Use exact `filePath` and
symbol selectors when known; otherwise search first, then use returned
selectors. Relationship queries need a compatible index and do not synchronize
it by default. Pass `syncIndex=false` only when that option is advertised.

When tracing a field across layers, query the identifier with
`harness_explore`; inspect `dataTrace` for the canonical field trace. A legacy
text response may also expose the compatibility alias `crossLayerTrace`.

## Source reads

`harness_read` uses inclusive 1-based `start` and `end` lines, up to 420 lines;
without bounds it returns up to 120 lines. Large reads can stop at the byte
budget and return `nextStart`. Continue from that line without rereading earlier
blocks.

`harness_symbols`, `action=read`, returns the exact selected source by default;
use `view=signature` only when the declaration is enough. `outline` lists
symbols. If a file has ambiguous names, use the container or `symbolId` returned
by the outline instead of guessing. A signature or summary is not a source
replacement.

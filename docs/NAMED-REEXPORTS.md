# Oxc named exports and reexports

Experimental Windows writer, existing schema 11/WAL index. No new MCP tool,
index creation, dependency downloads, Node runtime or active-connection switch.

## Implemented profile

Oxc 0.90.0 supplies `ParserReturn.module_record` and semantic root bindings.
The adapter retains owned export entries in the existing per-source semantic
cache; it does not parse export syntax with regex or build a second parser.
`oxc_syntax` is now a direct dependency at its already-locked version, without
introducing a new library/version into the dependency graph.

- Implemented top-level function declarations exported directly or through
  local named aliases: `function run() {}; export {run as api}`.
- Named indirect exports: `export {run as api} from './lib'`.
- Runtime named imports subsequently exported through a local alias.
- Chains of these forms, reusing the existing closed Oxc resolver and explicit
  tsconfig profile. No OS filesystem is supplied to the resolver.
- Only direct calls to an imported binding become call edges. Original refName
  and refKind survive alias changes so invalidation can reconstruct the reference.

The final graph target must be a unique top-level CodeGraph function in the
proven destination file. Its original is_exported flag need not be true when
Oxc proves a local export alias. Names in other files never win as a fallback.

Export-star traversal now extends this named profile; see
[STAR-REEXPORTS.md](STAR-REEXPORTS.md) for branching, ambiguity and provenance.
Not implemented: default/namespace binding, type-only
runtime calls, exported function-valued variables, CommonJS, callbacks/classes,
package resolution or nearest-tsconfig selection. Unsupported or missing targets
remain pending. Cycles naming no declaration remain pending; budget exhaustion
aborts publication rather than silently returning a complete result.

## Safety, provenance and invalidation

At most 16 active `(path, exported name)` pairs and 16 unique contributing files,
with 256 total visits per reference. Existing transaction
budgets still apply: 64 observed sources, 64 MiB source buffers, 16 MiB owned
semantic data, resolver probe/deadline limits and bounded reference inventories.
Every source is authorized, content-hash checked against the current transaction
and rechecked before commit. Changing a barrel during publication rolls back.

Native import edges retain their compatible resolvedBy marker and additionally
store `exportEngine=oxc-module-record-0.90.0-resolve-export-v2` plus bounded
`exportContributors` path/hash records for the chain, including the destination.
Publishing a JS/TS source invalidates/retries native import edges, including
those depending on a changed or deleted intermediate file. This is conservative
transaction-local rebinding, not a persistent incremental dependency planner.

Explore includes these contributors in its before/after freshness assessment,
not just the caller and callee. Recorded edge hashes are checked against both
index hashes and current content. Invalid, restricted, conflicting or oversized
provenance is rejected before reading its paths (16 entries/20 KiB per edge,
256 distinct contributor paths per query). Minimal legacy reader schemas without
edge metadata remain supported. No global freshness/coverage claim is added.

## Validation

`tests/reexports.rs` covers local aliases, chained renamed reexports, imported
bindings exported again, barrel-only edits, strict freshness, destination/barrel
deletion and recovery, cycles, type-only/default/namespace deferrals,
non-functions, stale intermediate sources, depth rollback and unsafe provenance.
A writer unit test mutates a barrel after SQL resolution and verifies rollback
of the target hash and existing call edge.

The opt-in `named_reexport_targets_match_full_provider_indexing` test indexes
six separate temporary projects with the CodeGraph v1.6.2 SDK and compares
actual caller target paths/names with native publication. It passed on 2026-10-17
for named reexports, renamed/local aliases and three star-chain cases. Node is used
only as the oracle; the native runtime never invokes it for this feature.

```powershell
cargo test --locked --test reexports
# With HARNESS_CODEGRAPH_PACKAGE and HARNESS_CODEGRAPH_NODE configured:
cargo test --locked --test reexports named_reexport_targets_match_full_provider_indexing -- --ignored --exact
```

Remaining work is explicit in PLAN-PENDIENTES-CODEGRAPH.md. These fixtures do not
certify complete ECMAScript/provider equivalence or rebuild unchanged legacy edges
automatically when the executable is upgraded.

Final verification on 2026-10-17: full locked workspace suite passed, strict
workspace/all-target Clippy passed, and the locked optimized Windows build passed.
The release executable also passed the existing explicit-tsconfig CLI fixture
with an empty PATH and reported version 0.7.0. Four stale verify expectations
were aligned with the previously merged TypeScript/Go plans (unit/protocol tests
only; no production verifier changes). Installed-toolchain/provider tests remain
opt-in unless explicitly selected; the named-chain provider oracle was selected
and passed separately.

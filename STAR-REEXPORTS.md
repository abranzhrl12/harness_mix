# Bounded export-star resolution with Oxc

Windows writer, existing schema 11/WAL index. Production still uses Oxc 0.90.0
and oxc_resolver 11.0.0; no dependency/toolchain upgrade, Node coordinator,
Rolldown bundler dependency, new schema or new MCP write tool was introduced.

## Reused components and rules

Oxc supplies runtime named/indirect/star module entries and semantic bindings.
The existing closed resolver supplies module paths. The adapter in
`src/adapters/index_exports.rs` adapts the canonical-binding, explicit-export
precedence and cycle rules exercised by Rolldown 1.2.14 and ECMAScript ResolveExport
to original CodeGraph function IDs. It does not parse syntax, generate a bundle
or replace the package resolver. Rolldown attribution/permission is retained in
`ROLLDOWN-LICENSE.txt`; no optimizer or tree-shaker is embedded.

- Explicit runtime exports win before considering star branches.
- Two paths to the same original `(file, local function name)` are one binding;
  two different original bindings are ambiguous, even in the same file.
- Cyclic paths alone yield no match; an independent valid branch can still win.
- A valid branch with no matching export does not block another branch.
- An unverified, missing or unsupported branch cannot be hidden by a successful
  branch. The call remains pending rather than selecting a convenient homonym.
- `default` is not inherited. Default/namespace binding and function-valued
  variables remain outside the native call-binding profile.

Type-only entries are erased from Oxc's runtime export view. The native kernel
still defers `export type *` with EXTRACTION_DEFERRED before publication; there
is no Node fallback. Named type exports combined with runtime stars are covered.

## Publication and freshness

Only direct calls through supported named imports resolve to unique original
function nodes. refName/refKind and the legacy native resolvedBy marker remain
recoverable. Newly resolved edges use
`exportEngine=oxc-module-record-0.90.0-resolve-export-v2`.

`exportContributors` is a sorted, deduplicated path/hash set covering every
verified branch consulted, including branches that currently export no matching
name. Such a branch can introduce ambiguity later without changing either
endpoint. Those files participate in pre-commit rechecks and Explore's existing
before/after freshness checks. Publishing JS/TS changes still invalidates and
retries native import edges, including ambiguity introduction/removal and deletes.

No context-dependent cross-branch negative memo is cached. Work is bounded by
16 active `(file,name)` pairs, 16 unique contributor files per reference,
256 total recursive visits and 256 runtime star entries per source. Existing
source/semantic/resolver/cancellation limits also apply. Exceeding a budget or
mutating any contributing source before commit rolls back the transaction.
Cycles/ambiguity stay pending; they do not produce a fabricated winner.

An engine upgrade still does not force a global rebuild: unchanged files may
take the existing no-op path. This is not an initial-index implementation,
whole-workspace synchronization or proof that unindexed alternatives do not exist.
Legacy named-chain edges remain readable; no active MCP connection is switched.

## Evidence (2026-10-17)

- Native matrix: 11 fixtures for direct stars, renamed diamonds, two forms of
  conflict, explicit shadowing, cycles in both orders and default exclusion.
- Same 11 fixtures matched Node's native ESM binding result and the result of
  executing Rolldown-generated synthetic bundles. Node/Rolldown are test oracles
  only; the native feature never executes user code or invokes either engine.
- Six positive named/star-chain fixtures matched complete CodeGraph v1.6.2
  indexing. Negative CodeGraph cases are not claimed equivalent.
- Tests cover non-winning-branch freshness, ambiguity addition/removal,
  deletion/recreation, stale/unsupported branches and bounded rollback.
- Writer unit coverage mutates a non-winning star branch after SQL resolution
  and verifies rollback of the target update and original call edge.
- Oxc-only unit coverage checks type erasure separately from kernel limitations.

```powershell
cargo test --locked --test star_reexports --test reexports
# Optional comparisons require explicit executable/package paths:
# HARNESS_CODEGRAPH_NODE: Node executable
# HARNESS_CODEGRAPH_PACKAGE: installed CodeGraph v1.6.2 lib directory
# HARNESS_ROLLDOWN_PROBE: built experiments/rolldown-linker-evaluation binary
cargo test --locked --test star_reexports star_binding_matrix_matches_native_esm_and_rolldown_generated_bundles -- --ignored --exact
cargo test --locked --test reexports named_reexport_targets_match_full_provider_indexing -- --ignored --exact
```

The isolated Rolldown probe accepts `--fixture-json` pairs for these fixed test
fixtures and still has no OS input fallback in its injected snapshot. This is
not a production JSON execution endpoint or a sandbox certification.

Final checks: the locked workspace suite passed, strict all-target Clippy passed
for production and the isolated probe, format checks passed, and the optimized
Windows build passed. The complete star test target passed all seven tests when
explicitly enabling the oracle and selecting the built release executable through
HARNESS_NATIVE_BINARY; its CLI published the original binding with PATH empty.
The isolated probe's 11 tests also passed. Tests use temporary indexes only.

The existing workspace index still reports provider extraction version 25 versus
27 after the CodeGraph 1.6.0-to-1.6.2 upgrade. Doctor recommends rebuilding it;
this feature does not rebuild or certify that real index automatically.

## References

- https://tc39.es/ecma262/2026/multipage/ecmascript-language-scripts-and-modules.html#sec-source-text-module-records-resolveexport
- https://github.com/rolldown/rolldown/blob/main/crates/rolldown/src/stages/link_stage/bind_imports_and_exports.rs
- [Named profile](NAMED-REEXPORTS.md)
- [Rolldown experiment](experiments/rolldown-linker-evaluation/README.md)

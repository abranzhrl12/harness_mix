# Sync extraction facts cache probe

This is a standalone cache model. It does not call `index_extract::extract`, open
the project database, touch `.codegraph`, or modify production code. Its tiny
declaration parser exists only to exercise keying, publish-after-success,
cancellation, and bounded LRU behavior. Passing tests validate this model, not a
production implementation or an engine speedup.

## Production facts observed

- `index_store/publication.rs::publish` skips extraction for an unchanged digest
  when configuration is unchanged; otherwise it calls `index_extract::extract`
  before publication and uses a transaction. This already avoids repeat parsing
  for duplicate unchanged watcher events.
- `index_extract/orchestration.rs::extract(path, source, context)` chooses the
  extractor by path extension and checks cancellation before and after native
  extraction. `decode.rs` checks cancellation while decoding nodes, and native
  extraction emits IDs / qualified names that are path-specific. Vue, framework,
  Razor, and template paths also construct path-bearing facts.
- Thus caching a complete extracted graph by content alone is unsafe across a
  rename. A reusable cache would need path-neutral parser facts, with path-bound
  IDs and fields materialized for each publication. This probe asserts that
  rebinding requirement with synthetic facts; it does not implement production
  materialization.
- Parser identity should be explicit in a production key. `Cargo.toml` pins
  `oxc_parser` to `0.90.0`, while other language paths use the vendored
  `codegraph_native` engine and specialized extractors. The probe's version token
  stands in for a composite extractor/parser build identity.
- Current extraction does not receive the tsconfig snapshot; tsconfig and other
  snapshots influence later resolution/publication. Including a normalized
  extraction-affecting configuration fingerprint is conservative and protects
  future/configurable fact extractors. It must not accidentally key facts on
  unrelated per-publication state.

## Run

```powershell
python experiments/sync-cache-probe/probe.py
```

The tests check repeat-hit extraction count, path-dependent IDs after rename,
content/language/parser/config misses, no entry after cancellation or parser
error, entry/byte budget eviction, and bypass for a fact set larger than budget.
One repeated lookup invokes extraction once in this model; that is a call-count
result, not a wall-clock performance result.

## Safe next step

Prototype path-neutral facts against the real extractor in a separate branch or
production-authorized task. First identify which emitted fields are path-bound
for every language, define parser/config identity, and measure realistic repeated
content across publications. Keep cache writes after the final cancellation and
error checks, and bound both entries and retained bytes. Then test rename,
configuration changes, parser-version changes, failures, and concurrent use
against the real transaction/publication flow before considering integration.

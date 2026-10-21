# Selective import invalidation probe

Run with `python experiments/sync-invalidation-probe/probe.py` from the repo root.

This deliberately small fixture model compares the current broad JS/TS recheck
scope (all importers in the fixture) with a candidate selective policy. The
policy must account for incoming resolved edges, unresolved module probes,
candidate-set changes on successful resolutions, transitive barrel/re-export
dependencies, and the importers that consumed a config snapshot. This is an
executable coverage/count probe, not a benchmark of SQLite, parsing, or the Rust
publication engine.

The current `unresolved_refs` rows hold reference name/kind/location but not the
module specifier or candidate paths. Therefore a selective implementation cannot
reliably find *negative module-path* dependents from that table alone. Existing
edges can find a deleted/current target, but an added alternate extension may
change uniqueness without targeting the new file. A practical next step is to
record each import's specifier, importer, resolver/config stamp, and the paths
probed (including absent paths), then test additions/deletions against those
records. Treat config changes as broad until config-consumer stamps are recorded.

An actual focused integration test also passes:

```powershell
cargo test --test index_store prior_import_edges_are_rechecked_when_exports_or_module_candidates_change -- --exact --nocapture
```

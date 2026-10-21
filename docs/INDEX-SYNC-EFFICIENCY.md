# Incremental synchronization: measured scope and safety

The native writer already skips extraction for an unchanged clean content hash
when configuration is unchanged. Changed files are extracted as whole files;
using RA syntax does not automatically make publication or Rust parsing incremental.

## Safe optimization in this change

Rust module invalidation recognizes legacy `syn-module-subset-v1` and current
`ra-module-subset-v1` edges. They can be collected in one metadata scan and
deleted in one statement, instead of scanning/deleting independently for each
engine. The resolver still retries both sets: this is not selective dependency
invalidation and does not remove correctness work for added or ambiguous modules.

The implementation retains per-engine row/retained-text budgets, reconstructs
original references, preserves unrelated/malformed metadata, and removes matching
orphan edges. Publication remains inside the existing transaction, with source,
configuration and scope rechecks, cancellation and rollback. WAL and FULL
durability are unchanged. No schema migration or index rebuild is required.
Both engine buffers coexist during collection: the retained text budget is at
most 32 MiB (16 MiB per engine), plus bounded collection/allocation overhead.
This can increase peak memory compared with collecting the engines sequentially;
the final publication still enforces its existing total reference limit.

## Verification layers

- SQLite unit contracts exercise the invalidator directly and compare SQLite VM
  work with the prior two-pass queries. This measures query work, not complete
  synchronization latency or a universal speedup.
  With 1,000 unrelated edges plus legacy/current/orphan/malformed cases, the
  two SELECTs executed 16,119 VM steps versus 10,095 for the production combined
  SELECT (about 37% fewer). The DELETE and reference inserts are not included
  in that measurement. Reproduce with
  `cargo test --locked -p agent-harness --lib optimization_tests -- --nocapture`.
- `tests/index_sync_robustness.rs` exercises the production Windows writer using
  temporary indexes: barrel ambiguity/recovery/export removal and explicit
  tsconfig alias changes from missing to resolved and between targets.
- Existing index-store and RA writer tests cover module candidate changes,
  legacy/current engine compatibility, unchanged extraction skips, and rollback.
- The three `experiments/sync-*-probe` directories are standalone illustrative
  models. Their passing assertions do not validate a production cache, pipeline
  or selective invalidation implementation. Synthetic timings are not a release
  performance guarantee.

## Deferred candidates

Selective invalidation needs persistent negative dependencies and candidate-set
observations (including absent module paths), transitive re-export dependencies,
and configuration consumers. Existing incoming edges alone are insufficient:
adding a new file can satisfy a previously missing import or make a unique
resolution ambiguous. Keep conservative invalidation until those inputs can be
tracked and tested against the real writer.

A cross-publication fact cache needs bounded retention and a correct identity:
current extracted records contain path-bound IDs, so content-only reuse across
renames is unsafe. Unchanged files already avoid extraction. Measure real cache
reuse before adding another cache.

Moving extraction before the write transaction needs owned facts, bounded queues,
and snapshot revalidation before publication as well as before commit. Keep one
SQLite writer; do not share parser arenas/CSTs across workers or weaken durability
to claim throughput gains.

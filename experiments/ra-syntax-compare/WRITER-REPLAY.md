# Rust resolver writer replay

This experiment compares the production `syn` facts provider with the isolated
RA call-facts candidate while both run through the real Windows
`index_store::sync_file` writer and temporary SQLite index. It is not a
production backend change.

`run-writer-replay.ps1` snapshots `src/`, `crates/`, `vendor/`, `Cargo.toml`, and
`Cargo.lock` into a unique directory under `%TEMP%`. It hashes the source inputs
before and after copying and aborts if they changed during capture. It adds the
same test-only replay module to that copy, runs the baseline first, then replaces
only the copied `src/adapters/index_rust.rs` with a shim for the pinned RA
candidate. The RA dependency and any resulting lockfile update are confined to
the temporary snapshot. Both runs use the same isolated target directory and
run sequentially.

The replay test publishes real route-like Rust module paths into temp SQLite and
compares serialized edge/pending/hash checkpoints after target edit, deletion,
re-add, module-path change, ambiguity creation/removal, and importer edit. It
also uses the existing test-only writer callback that fires after publication
and SQL changes but before scope/source rechecks and commit. The cancellation
and timeout tests assert that callback ran, the write failed with the expected
code, and the previous content hash and graph edges remain intact. A pre-cancel
or an unobserved timeout is not counted as a transaction rollback check.

Command:

```powershell
powershell -NoProfile -File experiments/ra-syntax-compare/run-writer-replay.ps1
```

The runner retains its temporary snapshot and logs its path, source manifest,
candidate hash, and checkpoint comparison for inspection. A replay result is
valid only if both engine runs completed and their checkpoint JSON matches.
Until a completed run is recorded here, incremental/cancellation parity remains
unverified.

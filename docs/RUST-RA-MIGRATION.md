# Rust parser: RA syntax integration

The native adapter now uses pinned `ra_ap_syntax = 0.0.289` for Rust syntax,
declaration navigation and the conservative free-function binding proof.
`syn` and `proc-macro2` remain development-only dependencies for frozen baseline
contracts and architecture tests, not runtime parser dependencies.

The shared `parsers/rust/syntax.rs` boundary preserves the 256 KiB limit,
conservative delimiter-depth budget, BOM/shebang normalization, operation checks
and strict rejection of syntax errors. RA recovery does not allow malformed edits
or partial graph facts. Parsing tries edition 2024 with a strict edition-2021
fallback to avoid rejecting previously accepted identifiers such as `gen`.
This API has no Cargo-edition context and is not a Rust compiler/type checker.

`symbols.rs` projects owned declaration records with unchanged UTF-16 IDs/ranges,
signatures, documentation and containers. `index_rust.rs` coordinates facts;
`index_rust/bindings.rs` and `calls.rs` isolate binding policy and call traversal.
Tests live separately. No macro expansion, receiver inference or new resolution
coverage is claimed.

New Rust edges use `ra-lexical-subset-v1` / `ra-module-subset-v1`. Rust module
invalidation recognizes both the new module tag and legacy `syn-module-subset-v1`
so a later authorized sync can rederive old relationships. No index is rebuilt
as part of this code migration and no schema/version change is needed.

The experiment retains frozen pre-migration syn sources under
`experiments/ra-syntax-compare/baseline/`. Production symbol tests compare against
that implementation, including explicit byte ranges. The writer lifecycle and
post-publication cancel/timeout rollback regressions run against the real native
writer with temporary SQLite stores, not the production index.

Earlier parse/extraction measurements are exploratory and do not establish an
end-to-end production speedup. See
[`experiments/ra-syntax-compare/CONTRACTS.md`](../experiments/ra-syntax-compare/CONTRACTS.md).

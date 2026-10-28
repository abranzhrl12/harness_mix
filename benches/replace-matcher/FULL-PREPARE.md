# Multiple replace: production preparation benchmark

Measured 2026-10-28 on the local Windows development machine. Raw measurements: [FULL-PREPARE-RESULTS.txt](FULL-PREPARE-RESULTS.txt).

## Decision and scope

Integrate a conservative Aho-Corasick lookup into `harness-domain::editing::prepare`. Select it only for normalized sources >=64 KiB, 16..=128 chunks, total old-text bytes <=4096 and average <=64 bytes. A cheap raw-input guard avoids normalization just to select a path. Other requests keep the sequential lookup. The first fragment still uses the original matcher to preserve cheap failures; if it requires whitespace tolerance, all remaining fragments use the original matcher. Missing exact matches use the original whitespace matcher. Automaton construction failure also falls back.

Standard overlapping matching detects ambiguity, duplicate patterns retain separate pattern IDs, and ranges remain UTF-8 byte offsets. Existing request/hash checks, overlap rejection, CRLF mapping, BOM handling, output bounds and no-op checks remain in place. Syntax validation and atomic publication are unchanged.

## Method

Run `cargo run --release --manifest-path benches/replace-matcher/Cargo.toml --bin full_prepare`. The benchmark compares the frozen pre-change preparation implementation in `src/reference/` against the live domain implementation, including exact output and error tuples across adverse fixtures before measuring. Each measurement uses seven samples with rotating engine order and calibrated rounds targeting at least 20 ms per sample. Automaton construction, supplied hash checks, normalization and output construction are included. AST validation, filesystem I/O and publication are excluded; these are not end-to-end tool speedups.

## Results

For LF inputs with 32 short unique fragments: roughly 96 KiB improved 3.80x, 512 KiB 4.56x (5.14 ms to 1.13 ms), and 2 MiB 4.35x (16.32 ms to 3.76 ms). At 128 fragments, 512 KiB and 2 MiB improved about 11.9x and 11.7x respectively. At 512 KiB/32 fragments CRLF improved 1.54x, BOM+CRLF 1.62x, mixed whitespace matching 1.36x. All-whitespace fallback was approximately unchanged (1.05x).

This is a synthetic corpus, not a universal speed guarantee. Bypassed cases also varied: 96 KiB/one fragment measured 0.886x and long fragments 0.823x, despite using the sequential lookup; compilation layout and measurement environment can affect timings. Small edits do not gain the multi-pattern algorithm. The original prototype's approximate eight-fragment crossover is therefore not used as the production threshold.

## Verification

New tests cover Unicode byte offsets, overlapping occurrences, duplicate/nested patterns, exact-match precedence, whitespace fallback, full BOM/CRLF output and stale-hash/NUL rejection. Differential full-preparation fixtures passed. Domain unit tests and all editing integration tests passed. The full domain run has one unrelated Git contract failure: `git_contract_tests::unified_git_rejects_missing_or_cross_action_fields_and_decodes_batches` at `contracts_compatibility.rs:357`; its missing-diff-path expectation conflicts with the existing default path. That contract and test were not changed for this optimization. Follow-up: the stale Git expectations were subsequently corrected to verify the existing workspace default for direct requests, batches and schema; the complete default-feature domain suite now passes. Final root-library verification: `cargo test --locked --offline -p agent-harness --lib` passed (448 passed, 7 ignored). Scoped `rustfmt --check` also passed. The installed executable was not replaced.

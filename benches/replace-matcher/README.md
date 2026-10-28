# Replace matcher prototype

Historical prototype measurements below. The subsequent production integration, conservative dispatch rule and full-preparation measurements are documented in [FULL-PREPARE.md](FULL-PREPARE.md). Statements below about unchanged production code describe the original prototype run.

Prototype isolated from the production API. It compares the current replace preparation in `harness-domain` with repeated exact `str::find` checks and an Aho-Corasick matcher using `MatchKind::Standard` plus `find_overlapping_iter`. The matcher is built inside every measured call.

## Reproduce

From the repository root:

```powershell
rustc --version
cargo run --release --manifest-path benches/replace-matcher/Cargo.toml
```

Measured on Windows x86_64, rustc 1.99.0 (b940084d7 2026-09-28). The run emits nanoseconds per operation. Each file size is the actual UTF-8 byte length (the last row is omitted if it would exceed the target). Chunks are distributed across the file. Runs use 100 iterations for 4 KiB, 20 for 64 KiB, and 5 for 512 KiB / 2 MiB.

## Representative observations

All values are ns/op. Columns: repeated exact lookup; Aho build+scan; Aho build+scan+exact output assembly; current full `prepare`.

| File bytes | Chunks | Repeated find | Aho build + scan | Aho exact + apply | Current prepare |
|---:|---:|---:|---:|---:|---:|
| 4,094 | 16 | 16,418 | 38,451 | 39,383 | 22,176 |
| 4,094 | 128 | 145,531 | 101,769 | 100,058 | 183,102 |
| 65,527 | 8 | 114,814 | 105,773 | 120,528 | 205,902 |
| 65,527 | 16 | 250,517 | 114,276 | 125,463 | 339,237 |
| 524,285 | 8 | 928,080 | 605,195 | 634,095 | 1,452,200 |
| 524,285 | 16 | 2,033,980 | 623,840 | 637,590 | 2,750,765 |
| 2,097,140 | 1 | 82,000 | 263,920 | 694,900 | 1,363,080 |
| 2,097,140 | 8 | 4,146,640 | 2,210,600 | 2,730,720 | 5,793,280 |
| 2,097,140 | 16 | 8,189,780 | 2,282,260 | 2,868,620 | 10,046,160 |

## Interpretation and limits

Building the automaton has a large fixed cost: it loses clearly for a single chunk, including on 2 MiB files. In this synthetic workload the matching-only crossover appears around 8 chunks at 64 KiB and 512 KiB, and around 8 chunks at 2 MiB; smaller files only show a gain at high chunk counts (64–128). These are candidate thresholds to investigate, not a production threshold.

The exact prototype checks byte-equivalent output against production `prepare` for every benchmark case. Its separate edge checks cover duplicate patterns, nested/overlapping patterns, UTF-8 boundaries, CRLF normalization, whitespace-tolerant fallback, missing matches, and ambiguity using production behavior as the oracle. No public API or production implementation was changed.

The timed Aho apply path covers exact matches only. It does not implement production error ordering, line targeting, CRLF offset conversion, whitespace-tolerant fallback, replacement EOL normalization, or all request validation. The benchmark therefore does not establish a safe production speedup. The timings are single-run wall-clock samples with few repetitions at large sizes and should be rerun on representative real requests before choosing a dispatch rule.

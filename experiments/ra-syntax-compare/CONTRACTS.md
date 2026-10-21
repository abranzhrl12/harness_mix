# Production symbol contracts and extraction measurements

This extends the parse-only experiment without changing production. The baseline
compiles the actual repository `syntax.rs` and `symbols.rs` through path modules,
using the actual domain Symbol model. The RA candidate is experimental Rust code
in `src/candidate.rs`, not an integration backend. Root Cargo files are unchanged.

## Reproduce

```powershell
cargo test --locked --release --manifest-path experiments/ra-syntax-compare/Cargo.toml --target-dir "$env:TEMP/harness-ra-syntax-compare-target" --test contracts --test call_syntax -- --nocapture
powershell -NoProfile -File experiments/ra-syntax-compare/measure.ps1 -Binary "$env:TEMP/harness-ra-syntax-compare-target/release/extraction.exe"
```

Four tests passed: ten declaration fixtures; all symbol fields in three real
repository files (29 symbols total); invalid-source/size/depth rejection; and
syntactic call/method-expression UTF-16 ranges. Byte ranges are checked explicitly
because production JSON omits them. Two real compatibility issues were found and
fixed in the candidate: CRLF doc comments and ordinary comments attached by Rowan
to declaration ranges. Fixtures also cover traits/impls, associated items,
negative impls, trait aliases, macros, raw identifiers, BOM/shebang and inner modules.

The benchmark gates exact symbol equality in a separate process before timing.
Both engines perform source hashing, parsing, symbol extraction, ID/signature/doc
projection and destruction. Syn includes the production span cleanup. Same release
binary/profile, three fresh process rounds per engine/size with alternating order;
each process uses three warmups and nine samples of five operations. Builds are
excluded. Median of process medians, milliseconds per extraction:

| Symbols | Production syn | Experimental RA |
| --- | ---: | ---: |
| 16 | 2.258 | 0.890 |
| 128 | 18.207 | 11.261 |
| 512 | 95.288 | 68.360 |

The observed process peak-working-set ranges were syn 1.98–4.01 / 4.96–5.23 /
8.65–8.76 MiB versus RA 3.75–3.91 / 4.50–4.70 / 6.45–6.59 MiB respectively.
These are whole-process observations sampled every 5 ms, NOT parser allocations,
RSS deltas, or guaranteed peak capture; particularly the smallest case is noisy.
Raw samples and source hashes: `extraction-results-2026-10-21.json`.

No other Cargo processes were observed just before these measurements; this does
not certify an idle machine. Fixtures are synthetic, not a representative full
workspace benchmark. All are below the existing size/symbol limits.

## Decision gate still incomplete

Call tests compare syntax and coordinates, NOT production `index_rust` binding
proofs, shadowing policy or persisted/incremental graph edges. No migration gate
is passed for graph resolution. Cancellation/deadline behavior, broader language
coverage, fuzz/adversarial input, larger files and full-workspace memory/throughput
remain unverified. The candidate preserves size/depth rejection, but is not a
replacement for production OperationContext and all extraction budgets.

These results justify continuing the evaluation, not silently switching the
production parser. The original parse-only README/results describe an earlier
capture; its historical lock hash predates the new isolated domain dependency.

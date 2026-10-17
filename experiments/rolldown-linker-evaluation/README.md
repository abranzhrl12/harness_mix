# Rolldown linker evaluation — isolated Rust probe

Decision: keep Oxc as the production base. Rolldown 1.2.14 successfully handles
the difficult ESM linking fixtures below, but its public output inspected here
is not a ready-to-store CodeGraph original-symbol/provenance map. Do not add the
whole bundler to the harness solely because these bundle tests pass.

This directory is an independent Cargo workspace/lockfile. No production
manifest, lockfile, schema, index or active MCP connection is changed. Existing
local edits outside this directory are not part of this experiment.

Final validation (2026-10-17): 11 tests passed, strict all-target Clippy passed,
format check passed, and the probe binary ran with PATH empty. This verifies
the isolated probe only, not the concurrently edited production workspace.

## What was exercised

The actual published Rust crates at 1.2.14 are used, not mocked behavior or a
Node wrapper. `BundleFactory::create_bundle_with_fs` receives an in-memory,
read-only snapshot and a matching custom-filesystem resolver. `generate()` runs
the real linker/code generator without writing a bundle. No user plugins are
loaded and generated JavaScript is never executed.

Fixtures cover:

- Renamed named exports followed by `export *`.
- Two different bindings with the same star-exported name: link error.
- Diamond paths to the same original binding: successful, not ambiguous.
- A star cycle with an independent valid branch.
- Default and namespace imports.
- Package `exports` conditions selecting ESM, not the CJS alternative.
- Inherited tsconfig paths, resolved from snapshot configuration.
- Missing/escaping imports, a real host file deliberately absent from the
  snapshot, invalid input paths and input budgets.
- Public metadata retaining the star marker and generated chunks omitting a
  barrel that still contributed to resolution.

The custom FS never falls back to OS reads and rejects all writes. Limits are
32 inputs, 256 KiB total source, 1000-byte relative paths and 100,000 FS probes.
This is a synthetic probe, NOT a production sandbox audit. The factory initially
constructs its OS-backed context before custom FS injection; built-in plugins,
configuration initialization, cancellation and all other paths would still need
review before adoption. Tsconfig auto-discovery is disabled for that initial
context. The async test timeout is not a hard CPU/process isolation guarantee.

## Important integration finding

For `app.ts -> barrel.ts -> leaf.ts`, the successful native probe returns:

```text
module_infos[barrel.ts].exports = ["*"]
module_infos[barrel.ts].imports = ["leaf.ts"]
output.chunk.sourceModules = ["leaf.ts", "app.ts"]
```

`ModuleInfo` exposes module IDs, import relationships and export names, not the
linker's canonical binding identity plus the full original reexport chain.
`OutputChunk` contains transformed/rendered output; the barrel can disappear.
Consequently, using chunk membership as CodeGraph provenance would omit a file
whose edit must invalidate the relation. Generated names/coordinates also cannot
be assumed to be CodeGraph IDs for the original source.

The private LinkStage has the richer resolved-binding state. Direct reuse would
need an upstream/public binding-snapshot hook or a maintained narrow extension,
plus our adapter for original IDs, authorized inputs, hashes and transactions.
This probe neither implements such a hook nor claims that no alternative API
could provide it. It establishes that the public module/chunk views tested here
are insufficient as a drop-in replacement.

Practical next step: retain the production Oxc parser/semantic/resolver, use these
Rolldown fixtures as an additional oracle, and evaluate a minimal resolved-binding
interface before deciding to embed/fork the bundler. Reuse the upstream linker
algorithm/tests when appropriate; do not create another parser or package resolver.
CodeGraph-wide framework rules, Rust/C# relations and index lifecycle are not
solved by this JS/TS linker experiment.

## Compatibility and measured dependency inventory

- Host compiler: rustc 1.99.0 (b940084d7, 2026-09-28), Windows.
- This experiment declares Rust 1.97, matching the highest declared dependency
  minimum observed. Only 1.99 was exercised, not an independent 1.97 MSRV test.
- Rolldown pulls Oxc 0.154.0 and oxc_resolver 11.24.3. Production retains Oxc
  0.90.0 / oxc_resolver 11.0.0 and its existing declared Rust minimum.
- Locked Cargo metadata: 275 packages in the probe including its root versus
  296 in the production workspace. 140 probe package/version pairs are absent
  from the production inventory. These include platform/dev resolution: they
  are NOT 140 measured runtime additions or a binary-size estimate. Upgrading
  production Oxc could change this comparison and would require its own tests.
- First isolated all-target check: 2m48s with two compilation jobs. The interrupted
  initial test build is not a reliable cold-build benchmark; subsequent command
  times include waiting for Cargo's artifact lock. Test execution itself is tiny.

No optimized-size or speed comparison with the production executable is claimed.

## Reproduce

Run from this directory:

```powershell
cargo test --locked -j 2 -- --test-threads=1
cargo clippy --locked --all-targets -j 2 -- -D warnings
cargo fmt --check
```

The binary emits the successful star-chain report. After building, it also ran
successfully with PATH empty (no Node/Deno/Rust toolchain lookup at runtime):

```powershell
$env:PATH = ''
& '.\target\debug\harness-rolldown-linker-evaluation.exe'
```

That smoke run observed 23 snapshot FS probes, 3 successful content reads,
7 denied reads and zero write attempts. Denied configuration probes are normal;
the real-host-file regression separately verifies the missing-module case.

## Sources and licensing

Registry sources and checksums are fixed by Cargo.lock. Rolldown is MIT; it also
documents upstream Rollup/esbuild attribution. No linker source was copied into
production by this experiment. If an extension is extracted, preserve notices.

- https://github.com/rolldown/rolldown
- https://docs.rs/rolldown/1.2.14/rolldown/
- Source reviewed: rolldown 1.2.14 `bundle/bundle_factory.rs`,
  `bundler/impl_bundler_getter.rs`; rolldown_common 1.2.14
  `types/module_info.rs`, `types/output_chunk.rs`, `types/rendered_module.rs`.

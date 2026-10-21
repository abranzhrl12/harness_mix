# RustCodeGraph adoption probe (not production)

An isolated executable calls the donor library and returns its extraction JSON.
It never opens a Harness index, writes a graph or configures MCP. Its own empty
workspace prevents native Tree-sitter/SQLite link conflicts with Harness.

The donor is fixed to `cfd4d74132219f498d5ae611d7e94990a27c2c0d` (1.2.10).
`Cargo.lock` fixes this probe's dependency resolution. It is a separate lock from
the donor CLI's upstream lock; do not treat the two executables as identical.
Their output paths are compared independently, with no performance comparison.

From the Harness repository root:

```powershell
$rcgBuild = Join-Path (Get-Location) 'target/refactor-audit/rustcodegraph-build'
cargo build --locked --manifest-path experiments/rustcodegraph-probe/Cargo.toml --target-dir $rcgBuild -j2

# Build the donor CLI from source in a NEW directory outside Harness's workspace.
# Never run its install/uninit commands on the real project for this experiment.
$rcgSource = Join-Path ([IO.Path]::GetTempPath()) ('harness-rcg-' + [guid]::NewGuid().ToString('N'))
git clone --no-checkout https://github.com/hunzhiwange/rustcodegraph.git $rcgSource
git -C $rcgSource checkout --detach cfd4d74132219f498d5ae611d7e94990a27c2c0d
cargo build --locked --manifest-path (Join-Path $rcgSource 'Cargo.toml') --bin rustcodegraph --target-dir $rcgBuild -j2

$env:HARNESS_RUSTCODEGRAPH_BINARY = Join-Path $rcgBuild 'debug/rustcodegraph.exe'
$env:HARNESS_RUSTCODEGRAPH_PROBE_BINARY = Join-Path $rcgBuild 'debug/harness-rustcodegraph-probe.exe'
cargo test --locked --test rustcodegraph_candidate -- --ignored --nocapture
```

The opt-in Windows test creates a disposable project, invokes `init -i` only in
that project, queries its DB read-only, and separately calls the library for the
same source fixtures. Each extraction/index command is bounded to 60 seconds.
The case oracle is independent of either implementation. A healthy simple case
and incremental CLI rename must pass before the remaining observations count.
Known differences are **reported**, so a green audit test does not mean that
`libraryAdoptionGatePassed` or `cliReplacementGatePassed` is true.

This is not a production adapter. No root dependencies, domain/application ports,
MCP schemas, existing identities or publication/freshness rules are changed.
Do not copy its graphs into `.codegraph` or switch the default backend based on
successful compilation. See [the pilot decision](../../docs/RUSTCODEGRAPH-PILOT.md).

The donor declares MIT; its notice is preserved in `UPSTREAM-LICENSE` for this
experiment. Review dependency notices before distributing an executable.

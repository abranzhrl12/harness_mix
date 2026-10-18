# Verification of PostgreSQL optimization, 0.8.1

2026-10-18, Windows x64, dedicated local PostgreSQL 18.6 fixture.

- `cargo check -p agent-harness --all-targets -j 2`: passed during implementation.
- `cargo build --release --locked -p agent-harness -j 1`: passed.
- Focused command with the explicit test environment:
  `cargo test --locked -p agent-harness -j 1 --test postgresql --test postgresql_pool -- --include-ignored --test-threads=1`:
  9 PostgreSQL tests and 2 pool/runtime tests passed, zero ignored or failed.
- Checks include TLS validation, hostname mismatch, changed CA content, current
  schema/permissions after ALTER/REVOKE, prepared statement/session reuse,
  configuration identity rotation, eight callers limited to four server sessions,
  timeout/discard/reconnect, cancellation/deadline while waiting for pool capacity,
  and invoking the synchronous API inside a current-thread Tokio runtime.
- The initial timeout test failed before acquiring its test lock because its
  cold localhost connection exceeded its one-second budget. The same case passed
  with explicit IPv4, and the fixture now uses HARNESS_PG_TEST_LOOPBACK_DSN.
  The one-second production timeout used by that test was not increased.
- `node scripts/smoke-mcp.cjs npm-dist/bin/agent-harness.exe`: passed against the
  exact packaged binary. Its 28 advertised tools matched the current shared
  contract; bootstrap, doctor, aliases, PostgreSQL status, jobs, cancellation and
  restart history passed. Concurrent unrelated workspace changes are preserved;
  this report does not certify all of those additional features.
- Real MCP benchmarks against the same verified-TLS fixture are recorded in
  [POSTGRESQL-PERFORMANCE.md](POSTGRESQL-PERFORMANCE.md), including raw samples,
  cold-start limits and both optimized trials. Final packaged warm medians were
  3.7441 ms (search) and 3.7251 ms (describe), versus 2117.6585/2147.0059 ms.
- The PostgreSQL fixture was stopped after verification. No global npm install,
  MCP configuration or credentials were changed.

## Artifact

`powershell -NoProfile -File scripts/package-npm.ps1 -SkipBuild` verified the
already-built optimized binary and produced
`target/dist/abranzhrl1212-agent-harness-0.8.1.tgz` (approximately 7.6 MB, 7 files).

Package SHA-256:

```text
84f5af3620c6e499b5764dd8a4b1f8694cd888bf8e24b43e531e7c71796735ae
```

Packaged executable SHA-256 (also recorded by the final benchmark):

```text
1e524c96c9c51e8d7e46f7057f0156fe350b4140c64a7d6deb02e84459ecd394
```

Publication remains blocked by npm authentication: `npm whoami` returned
`ENEEDAUTH` on the final check. The prior authorization to publish remains, but
the user must run npm login; no tokens should be sent through chat.

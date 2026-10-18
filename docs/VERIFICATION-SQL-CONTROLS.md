# SQL controls verification — 2026-10-18

Implemented SQL Server parameter binding, bounded streamed results, shared SQL
deadline/cancellation/redaction policy, consistent offline-by-default status,
sanitized diagnostic categories and truthful read-only/cancellation indicators.
See [SQL-CONTROLS.md](SQL-CONTROLS.md) for compatibility and scope.

## Final checks

Executed through real Harness MCP stdio against E:\harness_mix, with index watching
disabled and session 41b98cc4-3a58-44c3-b646-08cb3bb410f4:

- Initial workspace Rust build passed (job d96c4e0f-b82e-49c6-9517-430f5e9d4e67).
- Final SQL regression script passed (job 9cc924af-18cb-43fd-946f-ce81e0459219):
  9 policy/parameter/runtime unit tests; 4 PostgreSQL offline tests; 2 pool unit
  tests; 5 SQL Server offline/loopback tests; 5 opt-in PostgreSQL live tests.
  Total: 25 passing tests, no failures. The SQL Server live query test was not run.
- The final script recompiles the tested Rust targets; this is not only syntax
  verification. Harness reports verificationFreshness:not_tracked, not a global
  freshness certification or a complete workspace test-suite result.
- Harness JavaScript syntax verification passed for scripts/verify-sql.cjs.
- rustfmt --check passed for the changed SQL sources and tests.
- Actual MCP smoke passed against target/debug/agent-harness.exe: initialize,
  28 tools, bootstrap, doctor, aliases, SQL status, jobs, cancellation and restart
  history. The smoke script's generic "packaged executable" label does not mean
  this run tested or rebuilt an npm release artifact.

PostgreSQL live checks used only the previously created isolated fixture under
target/pg-catalog-test-92ce6a79cd274641b9bd337914477672, loopback port 54389.
They exercised catalog access, TLS validation, connection/prepared-statement
reuse, schema/permission freshness, four-session capacity and timeout/discard/
reconnect behavior. The temporary cluster was stopped successfully afterward.
The backend-noctalia project/database was not modified.

## Preliminary fixture failure

The first SQL regression run failed the synthetic stalled-handshake test: an
accepted Windows socket inherited the listener's nonblocking mode, returned
WouldBlock and closed early, producing SQL_CONNECTION instead of the expected
TIMEOUT. The fixture now explicitly makes the accepted socket blocking. The
final run passed the strict deadline and socket-closure assertions. The expected
MCP timeout state and envelope truncation location were also checked against the
actual result contract; no production timeout was relaxed.

## Remaining limitations

- Live SQL Server RPC execution requires an explicitly provided dedicated
  HARNESS_SQL_TEST_DSN; only driver-binding units, offline contracts and a local
  stalled TCP handshake were validated here.
- These changes add no PostgreSQL constraints/index/definition metadata, no
  SQL Server connection pool and no measured cold-connection improvement.
- Release/npm binaries were not rebuilt or published for this change. Existing
  version/package edits were preserved. npm whoami still returned ENEEDAUTH.
- No new performance multiplier is claimed; earlier PostgreSQL warm-connection
  benchmarks concern the previous pooling change, not this controls refactor.

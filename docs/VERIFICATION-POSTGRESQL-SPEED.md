# PostgreSQL speed and actual-endpoint diagnosis — 2026-10-18

## Real endpoint: measured, not simulated

The user authorized the PostgreSQL URI in E:\harness_mix\.env, variable
HARNESS_SQL_CONNECTION_STRING. The diagnostic loaded it into a private child
process environment without printing or persisting the value. The project
configuration still enables sqlServer, not postgresql; the URI is PostgreSQL.
No project/client/global configuration or server settings were changed.

At the requested endpoint, TCP connection measurements were 31.7804, 22.5120 and
12.9114 ms. These are connection establishment times, not SELECT execution times.
The strict-TLS Harness connection failed after 64.5571 ms with PG_OPERATION.
A separate unauthenticated PostgreSQL SSLRequest received byte N: this port does
not support TLS. No authenticated SQL queries ran against the real database.
The adapter did not downgrade to plaintext. A TLS endpoint or secure tunnel is
needed before measuring actual SQL through its supported connection policy.
The generic PG_OPERATION classification is a remaining diagnostic limitation;
the handshake probe, not that generic code alone, establishes the TLS blocker.
Raw sanitized timings: target/benchmarks/real-pg-before-20261018/real-diagnostic.json.

## Implemented optimization

- Connection pooling and prepared statements remain; no speculative library swap.
- Startup settings enforce default_transaction_read_only, search_path and bounded
  statement/lock timeouts. The adapter verifies them once after authentication,
  including when DSN options request unsafe values.
- Fixed, bounded catalog queries then need one exchange when prepared, not four.
  A new prepare adds one exchange; connection establishment and its one policy
  probe are separate cold work. Complete RowStream consumption through ReadyForQuery
  is required for reuse. Bounded excess rows are drained but not retained; errors,
  cancellation and timeouts discard the connection. This optimization is for
  fixed catalogs, not authorization to enable arbitrary PostgreSQL queries.
- Opt-in schemaCacheSeconds 0..300 (default 0): process-local describe snapshots,
  at most 64 entries / 2 MiB serialized payload, identity-scoped, cancellable
  per-key single-flight, no failures/empty/partial-result caching, explicit age,
  hash, TTL and refresh:true. Cache hits do not revalidate DDL or permissions.
- Existing harness_batch already supports SQL; descriptions now clearly advertise
  PostgreSQL/SQL Server with 1..4 jobs and at most two concurrent children.
- Configuration guides use the correct postgresql section and identify the
  environment variable without exposing its value. Contract version is 2.3.0.

## Controlled benchmark: not the user's server

Before/after executables are developer builds using the same optimized dev profile,
not npm release builds. Raw JSON artifacts contain executable hashes. The dedicated
local PostgreSQL 18.6 TLS fixture was accessed through a loopback proxy with an
artificial 20 ms delay per chunk in each direction. This is not a claimed 40 ms
production RTT. Ten samples/action; warm median excludes the first sample.

| Operation | Before | Final live | Final cached |
| --- | ---: | ---: | ---: |
| Search median | 251.75 ms | 62.75 ms | not cached |
| Describe median | 227.40 ms | 61.35 ms | 2.72 ms |

The final cached describe P95 was 35.55 ms, so this is not a worst-case/submillisecond
guarantee. Pool acquisition was about 0.004 ms in this fixture; network exchanges
dominated. The first connection remained around 2.6 s on this local TLS/localhost
fixture; its exact DNS/TCP/TLS/auth cause was not isolated. None of these timings
establishes the user's server's SQL latency.

Artifacts: target/benchmarks/sql-final-latency-20261018/{summary,before,after-live,after-cache}.json.
Earlier three-exchange intermediate trials are preserved under sql-latency-20261018,
sql-local-20261018 and sql-local-50-20261018. The no-added-delay intermediate live
results did not consistently improve locally; that prompted the final one-exchange
design. Those intermediate numbers are not measurements of the final executable.

## Verification and handoff

Final Harness job 523e6ea3-accf-4a76-96d0-f6dba1aa0ccd passed:
12 SQL unit tests, 9 contract tests, 5 PostgreSQL offline tests, 2 pool tests,
5 SQL Server offline/loopback tests and 8 real isolated PostgreSQL tests (41 total).
Live tests cover TLS, startup-policy enforcement, pooling, timeouts, concurrent
schema-cache misses, refresh, DDL/REVOKE visibility, TTL and truncated-result exclusion.
The SQL Server real-server test was not run. Verification freshness is not tracked
by Harness; this is not a full-workspace/global-freshness certification.

Final developer executable MCP smoke passed with 28 tools. The temporary local
PostgreSQL test cluster was stopped. No production data was modified. Package
version edits were preserved; npm/release artifacts were not rebuilt or published
for this worktree change. The real endpoint's TLS/tunnel choice remains blocked
on user direction, not on a proven slow database or pool bottleneck.

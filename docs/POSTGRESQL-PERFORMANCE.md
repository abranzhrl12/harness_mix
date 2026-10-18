# PostgreSQL catalog performance

Measured over the real MCP stdio interface against the same isolated PostgreSQL
18.6 server on Windows x64. The fixture uses TLS with a verified localhost
certificate and the dedicated harness_reader role. The table has two columns;
these measurements describe catalog latency, not arbitrary SQL or large datasets.

## Method

`scripts/benchmark-postgresql.cjs` executes 20 sequential search requests followed
by 20 describe requests. Each action reports its first request separately; warm
median and p95 exclude that first sample. Describe's first request may already
use the connection established by search. Raw samples are retained in JSON.

The previous optimized 0.8.0 executable was frozen before source changes:
`target/benchmarks/agent-harness-pg-baseline-0.8.0.exe`. Its SHA-256 is
`1f4006482afc3dd3d285e3c22b4428bb7f7231ff473d2f4c9e73c7e2e29af7cd`.

```powershell
node scripts/benchmark-postgresql.cjs target/benchmarks/agent-harness-pg-baseline-0.8.0.exe target/pg-perf-workspace target/benchmarks/postgresql-before.json 20
node scripts/benchmark-postgresql.cjs target/release/agent-harness.exe target/pg-perf-workspace target/benchmarks/postgresql-after.json 20
```

The fixture's connection value is provided via HARNESS_PG_TEST_DSN in the server
environment, never recorded by the benchmark.

## Measurements

| Operation | 0.8.0 warm median | 0.8.0 warm p95 | 0.8.1 warm median | 0.8.1 warm p95 |
| --- | ---: | ---: | ---: | ---: |
| Search | 2117.66 ms | 2238.29 ms | 3.74 ms | 5.16 ms |
| Describe columns | 2147.01 ms | 2303.99 ms | 3.73 ms | 4.40 ms |

Raw baseline: `target/benchmarks/postgresql-before.json`. It opened a new
connection for every request and reused no prepared statements. Raw optimized
packaged samples: `target/benchmarks/postgresql-after-packaged.json`. Search reused 19 of 20
connections and prepared statements; describe reused the connection for all 20
requests and its prepared statement for 19. The warm median improvement was
approximately 566x for search and 576x for describe on this fixture.

All 40 operations, including startup, took 86.04 seconds before and 2.48 seconds
after (approximately 35x overall). The first TLS connection was still expensive:
2.24 seconds before and 2.33 seconds after in this run. This measurement does not
show a cold-start improvement. The gain is amortizing connection establishment
across repeated requests; the first describe then took 9.19 ms. An earlier
optimized trial (`target/benchmarks/postgresql-after.json`) measured warm medians
of 4.12/3.67 ms and a 3.00-second first connection. Both trials are retained;
the table uses the final packaged executable, whose SHA-256 is
`1e524c96c9c51e8d7e46f7057f0156fe350b4140c64a7d6deb02e84459ecd394`.

Targeted checks cover TLS, CA replacement, current permissions and schema after
ALTER/REVOKE, eight parallel callers with at most four server sessions, and
discard/reconnect after a forced timeout. The one-second timeout fixture uses
explicit IPv4; localhost connection setup on this host exceeded that budget
before the blocking query had started. Pool wait deadline/cancellation and the
synchronous bridge inside a current-thread runtime are separately exercised.
The final focused suite passed after that fixture correction: 9 PostgreSQL tests
and 2 pool/runtime tests, no failures. The actual packaged executable also passed
the MCP smoke gate and exited cleanly with pooled sessions in the benchmark.

## Change and correctness boundaries

Version 0.8.1 uses one shared runtime and up to four total connections, including
idle sessions. Fixed catalog statements are prepared once per session. TLS and
authentication are performed when a new session opens. Only confirmed rollback
allows a lease to return to the pool; errors/cancellation/timeouts discard it.
Pool waits observe cancellation/deadlines, and idle/lifetime bounds limit retention.

Pool identities include the workspace, config content, credential fingerprint
and CA content. Credential material is not logged or used as plaintext keys.
Results and permissions are queried live on each request, so schema changes and
revoked privileges are not hidden by a metadata result cache. See
[POSTGRESQL.md](POSTGRESQL.md) for supported operations and session lifetime.

Concurrent unrelated workspace changes are preserved. The comparison measures
the complete executables on this local fixture; it is not a claim about every
network, dataset, proxy, authentication mechanism or deployment.

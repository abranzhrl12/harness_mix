# PostgreSQL catalog support

Harness 0.8.0 adds PostgreSQL to `harness_sql` with `engine: "postgresql"`.
The first implementation supports status, catalog search and column descriptions.
Arbitrary queries, routine definitions, defaults, comments, constraints and index
descriptions are not enabled in this release. SQL Server remains supported.

Add a `postgresql` section to `.agents/harness.config.json`:

```json
{
  "postgresql": {
    "enabled": true,
    "defaultSchema": "public",
    "allowedSchemas": ["public"],
    "maxRows": 50,
    "maxChars": 8000,
    "timeoutSeconds": 15,
    "connection": {"source": "environment", "name": "HARNESS_PG_CONNECTION_STRING"},
    "tls": {"mode": "verify-full"}
  }
}
```

Set the connection string in the MCP process environment, then reconnect the
client. Do not commit the value. TLS validates the certificate chain and hostname
using native platform trust. An optional `tls.caFile` must be a workspace-relative
PEM certificate path (at most 256 KiB). `disable-loopback` permits plaintext only
for explicit localhost/loopback hosts; it cannot be combined with a CA file.
Host-address overrides are rejected to avoid bypassing this restriction.

Use a dedicated role with CONNECT, schema USAGE and appropriate SELECT privileges.
The adapter does not grant permissions or change database objects. Metadata is
filtered by allowed schemas and the connected role's visibility. Empty results
are not proof that an object does not exist.

```json
{"action":"status","engine":"postgresql","connect":false}
```

`connect:false` (the default for both engines) checks configuration without reading the
connection environment variable or opening a network connection. `connect:true`
probes the database and reports sanitized failure codes. If both SQL Server and
PostgreSQL are enabled, every SQL operation must select an engine explicitly.

```json
{"action":"search","engine":"postgresql","query":"users","objectType":"table","schema":"public"}
{"action":"describe","engine":"postgresql","name":"public.users","sections":["columns"]}
```

Search accepts table/view/column/function/procedure. When objectType is omitted,
search covers relations. Routine results include identity arguments to distinguish
overloads. Describe currently accepts relations and the columns section only.
Identifiers containing dots require a future structured selector; use exact
ordinary schema.object names in this release.

Catalog statements are fixed and parameterized, executed in implicit read-only
transactions with server statement/lock timeouts and a local deadline. Version
0.8.1 reuses a process-local runtime, at most four connections in total (leased
plus idle), and at most eight prepared catalog statements per connection.
Read-only mode, search path and timeouts travel in the connection startup packet
and are verified once after authentication. Only completed consumption through
ReadyForQuery permits a connection to return to the pool. Errors,
cancellation and timeouts discard the session. Idle sessions expire after
60 seconds, checked by a 15-second reaper; sessions live at most five minutes.
Workspace, configuration, credential and CA-content changes select a new identity.
DSNs are not stored as pool keys or returned in diagnostics. Results stream within row and
output budgets; truncation is explicit. Raw DSNs and server errors are withheld.
If cancellation cannot be confirmed, the result says so and closes the connection.
Metadata passes through the existing heuristic redactor, which is not a universal
secret detector. By default every operation still reads PostgreSQL. Optional
`schemaCacheSeconds` (0..300, default 0) enables process-local describe snapshots;
it never caches status, search or arbitrary data queries. The cache holds at most
64 entries and 2 MiB of serialized payload. It excludes empty, failed or truncated
descriptions. Workspace/configuration/DSN/CA-content changes select a new identity.
Concurrent misses for the same object share a cancellable per-key gate.
`connectionReused` and `statementReused` expose actual live reuse, not cache hits.
`status(connect:true)` probes the selected live connection, which can be reused;
it is not a promise that an existing session was reauthenticated.

For repeated descriptions, merge `"schemaCacheSeconds":60` into the `postgresql`
section. A hit makes no database request and reports `schemaCache.hit`, `ageMs`,
`ttlSeconds`, `freshness:ttl_snapshot`, `permissionsRevalidated:false` and
`protocolExchanges:0`. DDL and revoked permissions can remain in an opt-in snapshot
until expiry. Use `{"action":"describe","engine":"postgresql","name":"public.users","refresh":true}`
to invalidate and replace it with a live read; failures never fall back to stale
metadata. Keep caching disabled wherever immediate permission freshness is needed.
`schemaHash` fingerprints returned metadata; it is not authorization or a query
guardrail. No cache claims global freshness.

Live catalog reads use the verified session policy instead of repeating BEGIN,
SET and ROLLBACK for every fixed read. A prepared statement needs one protocol
exchange; preparing a new statement adds one. Establishing TCP/TLS/authentication
and verifying the new session policy are additional cold-connection work.
Bounded catalog results are drained through ReadyForQuery even when output is
truncated, without retaining excess rows. Any incomplete/error response discards
the connection. `timings` separates pool wait, connection setup (including policy
verification), prepare and query/read; these
are server-adapter phase timings, not the complete client/model latency.

For independent tables use existing `harness_batch`, not sequential tool turns:

```json
{"jobs":[{"tool":"harness_sql","arguments":{"action":"describe","engine":"postgresql","name":"public.usuario"}},{"tool":"harness_sql","arguments":{"action":"describe","engine":"postgresql","name":"public.rol"}}],"session":"<bootstrap-session>"}
```

The names above are examples, not discovered tables. Discover actual names first.
Batch accepts 1..4 jobs with at most two concurrent children. Its separate reads
are not one shared transactional snapshot and parallelism does not guarantee half
the latency. PostgreSQL arbitrary action=query remains explicitly disabled.

Both engines use the shared result budget and cancellation policy documented in
[SQL-CONTROLS.md](SQL-CONTROLS.md). The result-array budget includes JSON punctuation
and escaping; `maxChars` is measured in UTF-8 bytes, excluding the response envelope.

## Integration test

Use a dedicated test database containing
`public.harness_catalog_fixture(id integer, label text)` and an environment
variable `HARNESS_PG_TEST_DSN`. The opt-in integration test uses loopback plaintext:

```powershell
cargo test --test postgresql -- --ignored --test-threads=1
```

Remote TLS tests require a test endpoint with a trusted certificate. Passing
offline tests alone does not certify a live PostgreSQL deployment.

The pool lifecycle tests additionally require `HARNESS_PG_TEST_ADMIN_DSN`,
`HARNESS_PG_TEST_LOOPBACK_DSN` (explicit numeric loopback, avoiding localhost
setup latency in the one-second timeout fixture), the
dedicated `harness_reader` role and an isolated test cluster. They create a unique
fixture table, test ALTER/REVOKE visibility and temporarily lock pg_attribute to
verify timeout/discard behavior. Never point these opt-in tests at a production
database. `scripts/benchmark-postgresql.cjs` records MCP cold/warm samples against
the same dedicated fixture and accepts a frozen executable for before/after runs.

The startup-policy test requires HARNESS_PG_TEST_POLICY_DSN pointing only at the
isolated fixture, with options requesting default_transaction_read_only=off,
search_path=public, statement_timeout=0 and lock_timeout=0. It verifies the adapter
overrides and checks those unsafe defaults rather than accepting them.

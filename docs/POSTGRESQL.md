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

`connect:false` (the PostgreSQL default) checks configuration without reading the
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

Catalog statements are fixed and parameterized, executed in READ ONLY
transactions with server statement/lock timeouts and a local deadline. Version
0.8.1 reuses a process-local runtime, at most four connections in total (leased
plus idle), and at most eight prepared catalog statements per connection.
Only confirmed rollback permits a connection to return to the pool. Errors,
cancellation and timeouts discard the session. Idle sessions expire after
60 seconds, checked by a 15-second reaper; sessions live at most five minutes.
Workspace, configuration, credential and CA-content changes select a new identity.
DSNs are not stored as pool keys or returned in diagnostics. Results stream within row and
output budgets; truncation is explicit. Raw DSNs and server errors are withheld.
If cancellation cannot be confirmed, the result says so and closes the connection.
Metadata passes through the existing heuristic redactor, which is not a universal
secret detector. Schema caching is not implemented; schemaCacheSeconds, if supplied,
is reported as requested but never represented as an active cache.
Every operation still reads PostgreSQL: result rows and permission decisions are
not cached. `connectionReused` and `statementReused` expose actual reuse.
`status(connect:true)` probes the selected live connection, which can be reused;
it is not a promise that an existing session was reauthenticated.

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

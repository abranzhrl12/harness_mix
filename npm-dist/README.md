# Agent Harness

Native Rust MCP server for Windows x64, distributed with a small Node launcher.

```powershell
npm install -g @abranzhrl1212/agent-harness
agent-harness --version
```

Configure your MCP client to launch `agent-harness` with
`["--root", "E:/absolute/project/path"]` over stdio. Restart the connection and
call `harness_bootstrap`, then reuse `data.session` in subsequent tools.

## Database setup: no JSON required

Put one connection string in the project's `.env` (never commit credentials):

```dotenv
HARNESS_SQL_CONNECTION_STRING=postgresql://USER:PASSWORD@HOST:5432/DATABASE
```

PostgreSQL URIs (`postgresql://` or `postgres://`) and SQL Server ADO strings
(`Server=HOST;Database=DATABASE;User ID=USER;Password=PASSWORD`) are detected
automatically. `HARNESS_PG_CONNECTION_STRING` and `DATABASE_URL` are also supported.
Existing process environment values take precedence over `.env`. No shell code
or variable interpolation is evaluated. Detection never opens a database connection.

Global npm installation does not create files in an arbitrary project. If you
want an optional editable configuration, run this inside the project:

```powershell
agent-harness --init-config
```

It creates `.agents/harness.config.json` without credentials and never overwrites
an existing file. Explicit database configuration overrides auto-detection,
including disabled engines. When both engines are available, select `engine`.
PostgreSQL TLS defaults to `disable` as configured by the user: remote traffic is
not encrypted. Set `tls.mode` to `verify-full` in the optional JSON to require TLS.

This release adds structured SQLite index diagnostics, a larger CLI sync budget,
compatible parameter aliases and initial PostgreSQL catalog access through
`harness_sql` (`engine: "postgresql"`). SQL Server remains available.

It also adds three planning and output tools:

- `harness_impact`: indexed inbound callers before changing a symbol. Supply
  `file` and `symbol`; declaration locations are not exact call sites. Inspect
  freshness, ambiguity and bounded coverage before editing.
- `harness_patch_preview`: the same hash-checked arguments as `harness_replace`,
  returning a unified diff and syntax diagnostics without publishing. Apply the
  reviewed replacement with `hash` set to the returned `baseHash`.
- `harness_observe`: recent retained job output, with `taskId`, a Rust regex
  `filter`, matching `lines`, `before`/`after` context and an incremental `cursor`.
  Output is bounded; inspect retention gaps and `omitted.truncated`.

Preview validates syntax, not types or compilation. Impact does not prove the
absence of dynamic references. Observe reads existing Harness jobs; it does not
start arbitrary servers or read arbitrary log paths. Restart MCP after upgrading
so the client refreshes its tool catalog.

PostgreSQL supports `status`, `search` and column descriptions with `describe`.
Arbitrary PostgreSQL queries and routine definitions are not enabled. The following
optional advanced JSON example enables certificate-verified TLS (credentials stay
in the environment or ignored project `.env`, not this JSON):

```json
{
  "postgresql": {
    "enabled": true,
    "defaultSchema": "public",
    "allowedSchemas": ["public"],
    "connection": {"source": "environment", "name": "HARNESS_PG_CONNECTION_STRING"},
    "tls": {"mode": "verify-full"}
  }
}
```

PostgreSQL `status` does not open a connection unless `connect:true` is supplied.
If both database engines are enabled, supply `engine` explicitly. Use a dedicated
reader role. `verify-full` validates certificates and hostnames; `disable-loopback`
remains available for restricting plaintext to loopback. No secrets or databases
are shipped in this package.

Index reads do not certify global freshness or full CodeGraph parity. Read the
returned diagnostics, coverage and truncation fields. Incomplete sync may have
confirmed per-file commits; do not infer workspace rollback.

Third-party notices for the CodeGraph kernel and Rolldown are included.

Harness reuses up to four PostgreSQL connections and prepared catalog
statements. Only a consumed ReadyForQuery returns a session to the pool; failed or
cancelled operations discard it. Idle sessions expire, and configuration,
credentials and CA content isolate connection identities. Query results remain
live by default. Optional schemaCacheSeconds caches describe snapshots with an
explicit age/TTL and refresh flag; cached permissions and DDL are not revalidated.

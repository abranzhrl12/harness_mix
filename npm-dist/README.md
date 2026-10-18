# Agent Harness 0.8.1

Native Rust MCP server for Windows x64, distributed with a small Node launcher.

```powershell
npm install -g @abranzhrl1212/agent-harness
agent-harness --version
```

Configure your MCP client to launch `agent-harness` with
`["--root", "E:/absolute/project/path"]` over stdio. Restart the connection and
call `harness_bootstrap`, then reuse `data.session` in subsequent tools.

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
Arbitrary PostgreSQL queries and routine definitions are not enabled. Configure
`postgresql` in the project's `.agents/harness.config.json`, use an environment
connection source, and restart MCP. Example (connection value stays outside files):

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
reader role; TLS validates certificates and hostnames. `disable-loopback` is an
explicit plaintext option only for local test servers. No secrets or databases
are shipped in this package.

Index reads do not certify global freshness or full CodeGraph parity. Read the
returned diagnostics, coverage and truncation fields. Incomplete sync may have
confirmed per-file commits; do not infer workspace rollback.

Third-party notices for the CodeGraph kernel and Rolldown are included.

Version 0.8.1 reuses up to four PostgreSQL connections and prepared catalog
statements. Only a confirmed rollback returns a session to the pool; failed or
cancelled operations discard it. Idle sessions expire, and configuration,
credentials and CA content isolate connection identities. Query results remain
live, including permission and schema changes; rows are not cached.

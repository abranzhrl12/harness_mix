---
name: harness-sql
description: Query SQL Server or PostgreSQL databases via Harness MCP, discover schema objects, and fetch bounded results. Not for editing SQL files as code or unsupported local SQLite databases.
---

# SQL with Minimal Sufficient Context

Read the [shared operating rules](../harness-workflow/references/operating-rules.md)
once per task. Check the live `harness_sql` schema. The database connection is strictly
read-only (`SELECT` / `WITH` only); Harness never executes `CREATE`, `ALTER`,
`DROP`, or write statements — schema migration scripts must be applied externally
(e.g., via SSMS or release pipelines). Never assume database engines,
credentials, or write authorization.

| Need | Action |
| --- | --- |
| Unknown connection state or engines | `status` |
| Unknown database object | `search` with a specific object name |
| Structure or definition of a known object | `describe` (`name="dbo.Object"` or `name` + `schema="dbo"`): automatically detects whether the object is a table/view (`relation`) or stored procedure/function (`routine`) without requiring a prior `search` or `kind` |
| Relationships within a schema | `map`, with bounded schema and limit |
| Query rows | `query` with `sql` (under `action=query`, `query` is also accepted as an alias) |
| SP / View / Function migration script | Draft and explain the SQL script; hand it off for SSMS or release deployment — never execute DDL/DML here |

Skip `status`, `search`, or `describe` when you already have a valid connection
and sufficient schema context. If two engines are configured, specify `engine`.
In PostgreSQL, omitting `schema` during search includes all schemas allowed by
policy; do not assume `public`.

- Use parameterized queries where supported by the connection and bound row
  counts using the engine's native syntax (`TOP` / `LIMIT`). Never invoke
  routines with side effects disguised as `SELECT`.
- Reuse valid metadata across calls. Caching depends on backend configuration:
  do not promise 0 ms latency or pass `schemaCacheSeconds` unless advertised by
  the tool schema. Use `refresh` only when supported and you need to invalidate
  stale metadata after external schema changes.
- Batch independent `describe` calls via `harness_batch` when supported. Never
  dump the entire database catalog routinely.
- For large stored results, use `harness_data_query` with JMESPath projections;
  fetch omitted details via `harness_detail` only when they affect the answer.
- Distinguish between zero matching rows, insufficient permissions, truncation,
  and runtime errors. Never improvise an alternative direct connection.

Conclude with the query result, applied filters, and relevant bounds; never dump
unnecessary sensitive data.

# Shared SQL controls

SQL Server and PostgreSQL share cancellation/deadline checks, redaction before
output budgeting, and a bounded row buffer. Drivers and connection lifecycles stay
separate: PostgreSQL retains its pool and read-only transactions; SQL Server closes
its per-operation connection and does not claim confirmed server cancellation.

## Status compatibility

Both engines now default to `connect:false`. This intentionally changes SQL
Server's old implicit probe. Callers requiring a network check must pass
`{"action":"status","engine":"sqlserver","connect":true}` explicitly.
Both report `not_configured`, `configured_unverified`, `ready` or
`verification_failed`, plus `engine`, `configured` and `connected`.
Failure codes are sanitized and retained, not replaced with a Boolean.

## Limits and safety

SQL Server clamps maxRows to 1..200, maxChars to 1000..32000 and timeoutSeconds to
1..120. PostgreSQL rejects out-of-range configuration, as before. Both enforce
the earlier of the request deadline and configured timeout. Checks interrupt
pending asynchronous connection, handshake and result reads, not just row loops.
Synchronous local config reads cannot be preempted by these async checks.

Despite its legacy name, maxChars bounds the serialized result array in UTF-8
bytes, including JSON escaping and punctuation, not the complete response envelope.
Rows beyond the budget are omitted and truncation is explicit in the MCP envelope
as `omitted.truncated:true` (the adapter internally returns `truncated:true`). A driver can
still allocate one large incoming cell; this is not a hard wire-level memory cap.
The existing redactor is heuristic, not a universal secret detector.

SQL Server streams results instead of collecting the full result set. It closes
the connection on completion, failure, timeout or early truncation; it does not
drain an unbounded result solely for reuse. Transport closure does not confirm
that server execution has stopped. Raw driver errors are neither returned nor
printed. Auth, permission, TLS, transport, lock timeout and deadlock failures have
distinct safe codes when available.

SQL Server arbitrary queries still use conservative lexical preflight, not a
server-enforced read-only transaction. Use a dedicated least-privilege login.
`readOnlyVerified` is now false; `readOnlyPreflightPassed:true` accurately describes
the check performed. PostgreSQL arbitrary queries remain disabled.

## SQL Server parameters

```json
{"action":"query","engine":"sqlserver","sql":"SELECT @id AS id, @label AS label","parameters":{"id":42,"label":"example"}}
```

Named scalar parameters (including P1/P2 names) are translated to Tiberius RPC
placeholders and bound, never value-interpolated. Keys may optionally start with
`@`, are case-insensitive and must be unique after normalization. Repeated SQL
placeholders share one bound parameter. Strings, booleans, signed integers, exact
representable decimals and null are supported; explicit CAST is advisable for
nulls because the binding uses a nullable string type. Arrays, objects, missing or
unused parameters, unsupported numeric ranges and ambiguous keys fail before
network access. Limits: 128 parameters, 64 KiB SQL and 64 KiB serialized values.
Placeholders inside literals, quoted identifiers and comments are not rewritten;
SQL Server global `@@` variables are preserved.

SQL Server relation describe now resolves ordinary schema.object names, defaulting
to the configured schema. Dotted/escaped identifiers need a structured selector
in a future version. Only the columns section is implemented; unsupported sections
fail explicitly instead of silently returning partial metadata.

## Repeatable verification

Run `node scripts/verify-sql.cjs` for policy/runtime unit tests and offline engine
regressions. It can also run as a Harness JavaScript test background job. Live
tests remain opt-in: set HARNESS_SQL_LIVE_TESTS=1 with HARNESS_SQL_TEST_DSN for a
dedicated SQL Server fixture, or HARNESS_PG_LIVE_TESTS=1 with the dedicated
PostgreSQL test variables described in POSTGRESQL.md. PostgreSQL admin lifecycle
tests mutate only their explicit fixture and must never target production.

These changes do not add PostgreSQL PK/FK/index/definition metadata or improve
first-connection setup latency. Those remain separate work; prior warm-connection
benchmark results do not measure these new changes.

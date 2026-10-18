# Agent Harness 0.8.1

Three new Rust MCP tools are included: `harness_impact` for bounded indexed caller
analysis, `harness_patch_preview` for hash-checked diffs and syntax diagnostics
without publication, and `harness_observe` for regex-filtered job output with
context, cursors and explicit retention/output limits. The Rust catalog contains
28 tools. Preview and impact are available in investigate/edit/full profiles;
observe accompanies background jobs in full profile. See docs/PLANNING-TOOLS.md.

PostgreSQL catalog performance: one process-local Tokio runtime, a bounded pool
of four total sessions, and prepared catalog statement reuse. Results are still
read live from PostgreSQL; no metadata result or permission cache is introduced.

A session is reusable only after confirmed transaction rollback. Errors,
timeouts, cancellation and dropped operations discard it. Pool waits observe the
request deadline/cancellation. Idle/lifetime bounds and workspace/configuration/
credential/CA-content identities limit retention and prevent cross-context reuse.

The result reports connectionReused and statementReused. A reproducible MCP
benchmark compares frozen executables using the same PostgreSQL TLS fixture.
See docs/POSTGRESQL-PERFORMANCE.md for measured results and validation.

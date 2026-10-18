# Verification of 0.8.0

Date: 2026-10-18. Platform: Windows x64.

## Observed checks

- `cargo check --all-targets -j 2`: passed before the concurrent workspace expansion.
- Directed tests (explore, index_diagnostics, llm_contract, postgresql, stdio):
  54 passed, with opt-in tests excluded. This includes an exclusive SQLite lock,
  schema version 999, malformed/missing databases and native synchronization of
  a source containing 1,200 references.
- Full Harness background verification:
  `b73b8fb0-566f-4378-978d-15c61682d4cb`, 843 seconds, exit 101. All executed
  functional suites passed. The only failure was the generated contract document
  still declaring 2.1.0 after the contract was changed to 2.2.0.
- After correcting `docs/TOOL-CONTRACT.md`,
  `cargo test --locked -p harness-domain --features schema -j 2`: 37 unit tests
  and 2 parameter-compatibility integration tests passed; no failures.
- PostgreSQL integration against an isolated PostgreSQL 18.6 cluster:
  `cargo test --locked -j 2 --test postgresql -- --ignored`: 2 passed. Covers
  catalog lookup, columns, overloaded routine signatures, restricted-role
  visibility, row truncation, TLS with a trusted test certificate, rejection of
  an untrusted certificate, and rejection of a hostname mismatch.
- The temporary PostgreSQL server was stopped after testing. Its diagnostic
  files remain under `target/pg-catalog-test-92ce6a79cd274641b9bd337914477672`.
- The updated executable was used through MCP on backend-noctalia with index
  watching disabled. `RegisterCreator` currently reports `database_missing` for
  `E:/PROYECTO RUST/backend-noctalia/.codegraph/codegraph.db`. This establishes
  the current condition, not the cause of the earlier recorded execution.
- Node syntax checks passed for the package verifier and MCP smoke script;
  PowerShell parsing passed for the npm packaging script.

## Scope and limits

PostgreSQL initially exposes status, catalog search and column descriptions.
Arbitrary queries and additional describe sections are explicitly unavailable.
No production PostgreSQL data was changed. No global MCP configuration or npm
credentials were modified.

During verification, concurrent changes introduced additional workspace crates
and replaced the Cargo manifest with an older version without the PostgreSQL
dependencies. The new workspace members were preserved, and the 0.8.0 version
and PostgreSQL dependencies were reapplied. The production package build selects
`-p agent-harness`; the earlier general test run does not certify the unrelated
new crates.

## Packaging/publication

`powershell -NoProfile -File scripts/package-npm.ps1` completed successfully with
the locked dependency graph and an optimized 0.8.0 executable. The package
verifier confirmed the executable/package version and required notices.

Artifact: `target/dist/abranzhrl1212-agent-harness-0.8.0.tgz` (approximately 7.5 MB,
7 files). SHA-256:

```text
7da6f67af3842e514e46ab9f8d114f3468d6f9adbd99e8804fd3239c652ef41c
```

The source release executable and npm-dist executable have identical SHA-256:

```text
1f4006482afc3dd3d285e3c22b4428bb7f7231ff473d2f4c9e73c7e2e29af7cd
```

`node scripts/smoke-mcp.cjs npm-dist/bin/agent-harness.exe` passed against the
actual packaged executable: MCP initialization, 25 tools matching the shared
contract, bootstrap, doctor, parameter aliases, PostgreSQL status, background
job completion/cancellation, and history after restart. `git diff --check` passed.

Publication has not occurred: npm returned
`ENEEDAUTH` both in the repository and in npm-dist. The user must run `npm login`
before the authorized publication can complete. Tokens must not be shared in chat.

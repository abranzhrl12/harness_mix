# Agent Harness 0.8.0

- Explore index failures include a structured indexDiagnostic with database path,
  stage, supported/observed schema versions and sanitized SQLite error codes.
  Busy/locked, corrupt and incompatible databases can be distinguished. Doctor
  retains this evidence instead of reporting a failed probe as healthy.
- CLI sync has a five-minute workspace budget and at most sixty seconds per file,
  capped by the parent deadline. Partial commits and remaining work stay explicit;
  failure reports now retain the message and deadline scope.
- Search accepts pattern as a query alias; read accepts lines as a count, mutually
  exclusive with end; batch accepts read/search aliases with canonical output.
- PostgreSQL catalog support in the unified SQL tool: explicit engine selection,
  status, relation/column/routine search and column descriptions, TLS validation,
  read-only transactions, server/local timeouts and bounded streamed output.
  Arbitrary PostgreSQL queries and additional describe sections remain unavailable.
- Cargo and npm use the same 0.8.0 version.

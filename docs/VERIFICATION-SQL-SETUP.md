# SQL setup verification — 2026-10-18

Implemented optional JSON configuration, PostgreSQL/SQL Server format detection,
process environment/project dotenv credential precedence, optional --init-config,
and the user's explicitly requested PostgreSQL TLS default disable. Contract 3.0.0.
No dependency was added and no global client/install settings were changed.

The internal credential route accepts only primary-project paths, retains existing
path traversal/symlink/reparse/size/deadline checks and returns no content through
normal tools. General file reads still reject .env. Credential/CA input is captured
once per operation so schema-cache identity and the actual connection share it.

Initial job f0692f18-c69c-4037-a622-87780ccb5f45 exposed the protected-dotenv
integration error; the normal reader was not relaxed. An internal credential-only
route fixed it. Subsequent jobs d8d385ba-6888-43cf-a6af-691b4e30dfd8 and
638c97e5-1b2c-4a99-b380-4176d5ece498 passed.

Final verification: 15 SQL unit tests, 4 project/path unit tests, 3 actual stdio MCP/
CLI setup tests, 9 contract tests, 5 PostgreSQL offline tests, 2 pool unit tests,
5 SQL Server offline/loopback tests and 8 dedicated PostgreSQL live tests: 51 passed.
The SQL Server real-server test remains opt-in and was not run. Verification
freshness is not tracked by Harness; this is not a full-workspace certification.
The standalone MCP smoke passed all 28 tool registrations and representative calls.
rustfmt --check passed for the changed Rust sources/tests. Local fixture stopped.

The actual user endpoint was also tested read-only after the user explicitly
authorized plaintext. In the final developer executable, a project with NO JSON
and no engine selector discovered the URI from its private process environment:
status SQL probes used the same connection, taking 18.08..30.08 ms warm, 119.18 ms
on first connection. Native psql SELECT 1 took 11.80..19.69 ms on that endpoint.
No configuration file was created. Raw sanitized measurements:
target/benchmarks/real-pg-auto-20261018/real-diagnostic.json.

Separately, real MCP calls against E:\harness_mix with the connection variable
explicitly absent from the child process successfully used the project's .env,
reported connected:true/tlsMode:disable, and reused the connection on the second
probe. No credentials or real database rows were printed or persisted.
An earlier native diagnostic passed a URI as PGDATABASE and failed authentication;
the helper was corrected to explicit private PGHOST/PGPORT/PGDATABASE/PGUSER/
PGPASSWORD environment fields. No database passwords were guessed or changed.

Only SELECT/session-local settings ran on the user's database; fixture DDL and
locking tests ran exclusively in the isolated local test cluster. No production
data or server configuration was modified. Existing project SQL settings were
preserved except the authorized switch from SQL Server to PostgreSQL and TLS off.

npm whoami returned ENEEDAUTH. No npm publication or optimized release rebuild
was performed for this change; npm @latest and the globally installed executable
must not be represented as already containing it. The tested artifact is
target/debug/agent-harness.exe (optimized dev profile). Other pre-existing Cargo/
package/binary edits were preserved.

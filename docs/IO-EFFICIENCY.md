# I/O efficiency and release checks

## Creation and confirmation in one call

`harness_write` keeps its path/content request. New optional booleans default to
false: `createParents` and `allowUnvalidatedText`. Parent creation is bounded to
32 levels, with pinned ancestor handles and rejection of reparse points, links,
protected metadata/instructions, and secret paths. Content and syntax are checked
before creating directories. Existing equal bytes return `already_present` without
writing; different bytes conflict. Newlines are compared byte-for-byte.

```json
{
  "path": "src/entities/profile.rs",
  "content": "pub struct Profile;\n",
  "createParents": true,
  "session": "<bootstrap session>"
}
```

The result includes status, changed/created, commitStatus, hash, verified, syntax,
changeSummary, createdDirectories, filesystemChanged, and typecheck=not_run.
No source echo or build is needed to confirm publication. Replacement also
returns byte/line counts and net line delta, not a claimed minimal diff.
Preview remains a separate, optional pre-publication operation.

`allowUnvalidatedText=true` is an explicit opt-in for formats with no parser.
Such a file returns syntax.status=not_checked. It does not bypass syntax errors
or parser limits of supported languages. JSON creation failures include the
parser's line/column; other parsers retain their existing diagnostic precision.
Vue edits validate inline JavaScript/TypeScript scripts only; template and style
blocks are explicitly reported as unchecked. No inline script returns
`syntax.status=not_checked`.

Edits under `.agents/knowledge/` are a narrow exception: only Markdown documents
inside that subtree may be replaced or created. Other `.agents` files, including
`AGENTS.md`, remain protected. Creating a knowledge document requires its parent
directory to exist; `createParents` does not open other `.agents` paths.

If an I/O failure or cancellation happens after directories were created, the
response lists them and reports directories_retained. No unsafe recursive
rollback is attempted. Files are never overwritten, and a successful commit
is not hidden by a later cancellation. Directory-only side effects invalidate
session verification observations. Multi-file atomic transactions are not implemented.

## Compact bootstrap

`harness_bootstrap` accepts `compact=true`. The initial catalog is moved to
recoverable detail, but project rules, usageRules, safety limitations, execution
policy and persistence warnings stay visible. Large rules and catalog detail
are merged rather than overwriting one another. Without this flag, legacy initial
output stays available. Reusing the same session continues to use the existing
hash-aware compaction; do not bootstrap before each operation.

## Limits and distribution

`harness_read` accepts up to 400 lines (`start` plus `end`, or `start` plus
`lines`). Its existing 16000-byte snippet budget remains unchanged. Larger
requests fail explicitly; source is never silently truncated. For one specific
implementation prefer `harness_symbols` with `action=read` and `view=full`.

Read, write and replace accept `target_path` as an alias of `path`, and
`sessionId` as an alias of `session`. Canonical output and documentation keep
`path` and `session`. Supplying both names for the same field is an error,
not an ambiguous precedence rule. These aliases are scoped to these I/O tools;
other tools retain their advertised contracts.

`npm publish` from `npm-dist` runs the repository's package preparation script:
build the locked release, compare Cargo/npm/executable versions, copy the
executable, and probe its real MCP tools and schemas. Missing `harness_write`,
mandatory replacement hashes, missing replacement chunks, or a missing Rust
verification `package` parameter stop publication. Preparation failure also
stops publication. This workflow requires the repository and its toolchain;
publishing a detached npm-dist directory is deliberately not supported.

The ordinary packaging command still creates a tarball. `-PrepareOnly` prepares
and verifies without running `npm pack`. `-SkipBuild` remains an explicit local
packaging option and is not passed by the publication hook. npm's explicit
`--ignore-scripts` can bypass lifecycle checks and must not be used to publish.

Run the small verifier regression tests with:

```text
node --test npm-dist/scripts/verify-package.test.cjs
```

The verifier additionally requires creation options and compact bootstrap in the
packaged schemas. An old artifact correctly fails that check until rebuilt.
Schema checks alone do not prove behavior: run creation/compact bootstrap and
read-range regression tests on the final build before distribution.

The existing global installation is not modified by these source changes.
Clients may resolve a .ps1, .cmd, or standalone .exe with the same command name.
Inspect the running server's executable path and `harness_bootstrap` version,
not just the version returned by a different shell command.

## Implementation verification (2026-10-18)

78 directed tests passed: creation workflow (9), existing friction (3), existing
LLM efficiency (3), session workflow (5), domain (38), application (16), creation
race/cancellation unit test (1), and npm verifier tests (3). The creation unit test
injects cancellation and a concurrently created destination before publication,
checks pinned parent handles, and verifies temporary-file cleanup and explicit
retained-directory reporting. This is not a full-workspace suite certification.

Also checked all agent-harness test targets with cargo check and probed the debug
executable's real MCP tool schemas using the package verifier. No optimized release
was rebuilt, no npm package was published and no global installation was modified
in this implementation step. Rebuild and validate the final artifact for the next
release, choose a new package version before publishing, and reconnect the client.

Main verification commands:

```text
cargo check --locked -p agent-harness --tests
cargo test --locked -p agent-harness --test create_workflow --test friction --test llm_efficiency --test workflow
cargo test --locked -p agent-harness --lib adapters::create_file::tests::
cargo test --locked -p harness-domain -p harness-application
node --test npm-dist/scripts/verify-package.test.cjs
```

# Explicit tsconfig paths/baseUrl profile

Windows publication, existing schema 11/WAL index only. No creation/migration of
an index, no source writes, no switch of the active MCP connection.

```powershell
cargo run --locked -- --root E:\project --sync-file src/app.ts --tsconfig tsconfig.json
```

The first selection must exist. One selected configuration applies to the
index's supported JS/TS import subset, not automatic nearest-project discovery.
Later sync-file calls reuse that explicit selection. Sync revisits one unchanged
JS/TS file when its source plan is empty, so configuration-only changes/removal
can invalidate relationships. Removing the selected configuration clears its
selection and returns its imports to pending; recreation requires explicit
selection again. Transactions are per source, not per workspace.

## Supported profile

- UTF-8 JSONC with optional BOM, line/block comments and trailing commas.
  Strings remain intact; JSON5 syntax and malformed/unterminated comments fail.
- compilerOptions.paths: exact keys or one wildcard, ordered fallback targets.
- baseUrl anchors those targets relative to the declaring file. Without baseUrl,
  targets are relative to that file's directory. Normalized `..` may stay inside
  the workspace; absolute paths, traversal escapes and restricted paths fail.
- Named imports/direct calls and the function export profile described in
  [NAMED-REEXPORTS.md](NAMED-REEXPORTS.md), including named reexport chains.
  No default/namespace/type-only runtime calls are added.
- Unmatched bare packages, Node builtins and missing destinations remain pending.
  baseUrl alone does not turn arbitrary package names into local symbols.
- Relative imports keep their previous behavior. Conservative directory-index
  deferrals remain; this does not assert full TypeScript/provider equivalence.

- One local `extends` string (`./` or `../`, optional `.json`) per config is
  followed inside the workspace. Nearest paths replace the entire inherited map;
  baseUrl and paths keep the directory of their declaration. An overriding baseUrl
  anchors inherited paths as in the compared CodeGraph profile. Missing parents,
  cycles, arrays and package-based inheritance fail closed.

references and module-resolution options outside this profile are not
followed. Explicit unsupported options (rootDirs, moduleSuffixes, customConditions,
pathsBasePath) fail closed. Other compiler settings do not become resolver rules.
Nested project selection, package manifests and node_modules are not enabled.

## Safety and invalidation

Configuration is read through Project validation, capped at 256 KiB, with Windows
parent/file pins and rejection of reparse points/hardlinks for every ancestor.
At most 8 configuration files and 1 MiB combined original input; cycle detection
folds ASCII case (the depth limit also bounds other spelling variants). At most 128 mappings,
8 targets per mapping, one wildcard, 1000-byte paths and 64 normalized components.
No parser error or raw configuration is returned in failure results.

Only validated target paths are converted into a sanitized virtual configuration.
Oxc 11.0.0 receives it through the existing closed SQLite filesystem snapshot,
never an OS filesystem. baseUrl is flattened into those targets; it is not given
to Oxc as an implicit bare-package fallback. The pinned resolver's manual paths
context uses the validated virtual configuration directory. Existing resolver
probe/memory/deadline limits and source-observer budgets still apply.

project_metadata.native_tsconfig_v1 stores the selected relative path, SHA-256,
and the relative paths/hashes of inherited configurations (bounded to 16 KiB).
Native import edges also carry that provenance. A changed selection/hash bypasses
the source no-op, recovers native import references and re-resolves them in the
same transaction. The configuration and contributing sources are rechecked before
commit, including all inherited configs; mutation or unsupported input rolls back.
No schema tables are added.
Explore includes configuration hashes in its before/after freshness assessment,
so unchanged source files alone cannot certify relations after config edits.
This is still not an atomic filesystem snapshot or global freshness certificate.

## Validation

Tests cover exact/wildcard/fallback targets, base anchoring, relative precedence,
unmatched bare imports, config-only changes/removal, strict freshness rejection,
invalid/oversize/restricted/escaping configurations, hardlinks, missing selection,
pre-commit mutation rollback and CLI publication with an empty PATH (no Node).
An inherited NODE_PATH cannot bind a missing alias to an unrelated indexed file.
The same CLI fixture can validate a built release executable without Node:

```powershell
$env:HARNESS_NATIVE_BINARY = (Resolve-Path target/release/agent-harness.exe).Path
cargo test --locked --test tsconfig_paths cli_exposes_explicit_selection_without_node
Remove-Item Env:HARNESS_NATIVE_BINARY
```

This environment variable selects the executable only inside the test; it is not
a runtime harness configuration setting. The child runs with an empty PATH.
Additional tests cover JSONC strings/invalid syntax, parent freshness and override
behavior, cycles, missing ancestors, depth/aggregate byte budgets, ancestor
hardlinks and parent mutation before commit. Changed Rust files are formatted
separately to preserve upstream/vendor formatting. SQL/XLSX collapsible-if warnings
were corrected without changing their behavior; strict workspace Clippy passed.

The opt-in differential fixture is
`alias_targets_match_installed_provider_for_supported_cases`, configured with
HARNESS_CODEGRAPH_PACKAGE (v1.6.2) and HARNESS_CODEGRAPH_NODE. It compares native
targets with the provider's alias loader/import resolver. Executed successfully
on 2026-10-17 against installed v1.6.2 for exact/wildcard/fallback/missing aliases,
JSONC, local inheritance, and paths/baseUrl overrides (six differential cases).
This proves those fixtures, not full provider parity. Upstream was reviewed at the pinned
[CodeGraph source](https://github.com/colbymchenry/codegraph/blob/6560052a6f856855d3f71eee838fd66ccfa4285d/src/resolution/path-aliases.ts).

The full locked workspace suite passed on 2026-10-17, as did strict Clippy with
all targets. Provider comparisons for extraction, import mappings/unambiguous
paths and relative-resolution priorities also passed against installed v1.6.2.
Provider-only and installed-toolchain tests remain opt-in in the ordinary suite.
The locked optimized Windows build also passed. Its 26,465,792-byte
`target/release/agent-harness.exe` passed the explicit-selection CLI fixture with
an empty PATH, as well as `--version` (0.7.0). This validates the profile without
Node, not the still-pending full CodeGraph lifecycle or all runtime capabilities.

Remaining: multiple-parent/package inheritance, per-importer selection, references, package
resolution, exports/reexports and broad differential compatibility.

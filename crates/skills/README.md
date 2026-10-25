# harness-skills

Reusable skill synchronization for Harness projects. The crate embeds the six
repository skills, tracks managed files with a SHA-256 manifest, preserves local
edits, and supports preview/check and opt-in startup synchronization.

The implementation is split into private modules: `catalog` owns embedded
sources, `sync` plans and applies synchronization, `manifest` tracks managed
hashes, `preferences` handles the opt-in setting, `storage` enforces safe file
access, `lock` serializes writes, and `report` defines the public result types.
The library facade exports only the sync entry points and report/options types.
The CLI initialization workflow uses the documented `project_support` API;
database and MCP example configuration remain owned by the CLI.

The CLI owns project initialization and its database/MCP example files. This
crate only provides the skill catalog, safe file operations, synchronization,
and auto-sync preferences. It depends on `harness-domain` for hashing and the
tool contract version; it has no CLI, MCP, application, or async-runtime
dependency.

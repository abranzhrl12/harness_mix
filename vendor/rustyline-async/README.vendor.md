# rustyline-async local fork

This directory contains the minimal build sources for `rustyline-async` 0.4.9,
vendored from the Cargo registry package `rustyline-async-0.4.9` and licensed
under the included `UNLICENSE`.

Upstream: https://github.com/zyansheep/rustyline-async
Crates.io source checksum: `b76450411764ae23570aa50bae870dac68db933268df2ab3a577d641299d93a2`

The local changes add opt-in Ctrl+V / Ctrl+Alt+V paste request events and an
API for inserting clipboard text into the active line without submitting it.
Both clipboard insertion and terminal paste events normalize newlines and
remove C0/C1 control characters (while preserving tabs) before rendering.
`src/lib.rs`, `src/line.rs`, `src/history.rs`, and `UNLICENSE` are retained from
the upstream package. `Cargo.toml` keeps the runtime dependencies and omits
example and development-only targets; upstream examples and other development
assets are omitted.

# `harness-cli`

`harness-cli` owns command parsing, preparation, root resolution, and native workspace discovery. It parses an argument vector into typed `Command` and `Root` values, including legacy index aliases, skills commands, and server options. The crate has no runtime dependencies.

This is a partial extraction, not a migration of the whole executable. The native host still dispatches prepared commands and owns server, skills, configuration, and index execution. Keep command execution outside this crate.

The host can consume the public API with `parse`, `needs_workspace_root`, `Command`, `Root`, `prepare_command`, and `resolve_root`. Parser behavior is covered by `cargo test -p harness-cli`. This separation is for dependency direction and independent testing; it makes no runtime speed claim.

# harness-config

Runtime configuration shared by the native server entry points. The crate root
is a small facade that re-exports `RuntimeConfig`; the implementation is split by
responsibility:

- `src/runtime.rs` defines runtime options and parses the `from_values` snapshot.
- `src/environment.rs` reads process environment values for `from_env`.
- `src/tests.rs` contains unit tests for parsing and platform policy.
- `tests/architecture.rs` protects the facade and dependency boundaries.

`RuntimeConfig::from_values` accepts an environment snapshot so parsing and
platform defaults can be tested without changing process environment variables.
The public `from_env` and `from_values` APIs and their existing parsing behavior
are preserved.

This crate does not construct services or load project/database configuration.

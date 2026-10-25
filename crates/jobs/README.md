# harness-jobs

Durable background verification jobs, independent of the native executable, CLI
and MCP transport. The host injects `VerificationExecutor` and `DirectoryPinner`
when constructing `TaskManager`; `JobService` and event-stream contracts remain
in `harness-application`.

## Modules

- `lib.rs`: public facade.
- `engine.rs`: admission, scheduling, retained state, journal and shutdown.
- `execute.rs`: worker execution, retry policy, cancellation and result handling.
- `observe.rs`: bounded views of retained output.
- `store.rs` and `store/`: SQLite persistence, snapshot validation and safe paths.
- `engine_tests.rs`: scheduler and recovery tests with injected executors.

The native adapter prepares language-specific verification plans and annotates
their results. Plans are validated at admission and prepared again immediately
before each execution attempt. The job engine never builds native verifier
commands or imports language parsers.

The existing fixed-path database, schema, single-writer lock, bounded history
and fail-closed persistence behavior are preserved. On Windows, the injected
directory pinner must retain ancestor handles that deny rename/deletion for
the lifetime of the store. This crate does not migrate or delete user history.

`shutdown_and_wait` cancels admitted jobs and waits for workers to drain within
the caller's deadline. A timeout is not proof that workers have terminated.

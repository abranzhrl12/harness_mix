# `harness-cli`

`harness-cli` owns the command-line interface: argument parsing, typed command requests, command dispatch, CLI progress and reporting, skills management, and interactive terminal flows. The parser core (`domain`, `application`, and `adapters/input`) stays independent from host execution, async runtimes, and terminal implementations so it remains easy to test and reuse.

Execution enters through explicit ports. The host provides implementations for project operations and interactive conversation creation; CLI handlers depend on those contracts and on application/domain APIs, never on the root `agent-harness` package. This keeps the dependency direction one way and allows the executable host to compose CLI behavior with the existing adapters.

Keep terminal I/O and command-specific handlers in their respective modules as the command set grows. Add a new command by defining its parsed request, registering its handler, and extending the host port only for capabilities that need an implementation outside this crate. Avoid a global service locator or a command registry framework until runtime extension is an actual requirement.

Parser behavior and the layer boundaries are covered by `cargo test -p harness-cli`. The crate boundary supports independent evolution and testing; it makes no runtime speed claim.

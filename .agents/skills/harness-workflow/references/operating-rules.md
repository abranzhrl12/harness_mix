# Shared Harness operating rules

Read this once per task that uses Harness MCP. Reuse its guidance; task-specific
skills add only their own tool details.

## Session and scope

- Use the connected tools' live names and schemas. Bootstrap only when there is
  no valid connection/session; verify the selected root or every registered
  project root, then reuse `data.session`.
- Keep reads and searches within the user's authorized workspace and scope.
  `permission to investigate` does not authorize edits, builds, external access,
  or index maintenance.
- Reading and searching need no index. Relationship queries require a
  compatible index; initialize, sync, or rebuild only with explicit
  authorization. Never delete an index to repair it.

## Evidence and failures

- Preserve hashes, freshness, coverage, truncation, omissions, diagnostics, and
  `detail.ref` when they affect the conclusion. A bounded empty result does not
  prove absence. A summary or signature is not full source.
- A transport, timeout, or runtime failure is a stopping condition for
  equivalent retries: diagnose once with `harness_doctor` and report the
  blocker. An unsupported operation is a limitation, not a transport failure.
  Do not bypass a Harness failure through shell or another channel.
- Make success claims only from fresh evidence that supports that exact claim.
  Report checks as `validated` or `pending`; distinguish syntax checks,
  focused tests, builds, and runtime evidence. Follow the project's allowed
  test scope.

## Changes and external effects

- Preserve unrelated and concurrent changes. Use only authorized write tools
  and current source context. Permission rejection never grants permission to
  try another route.
- Do not change client configuration, credentials, or permissions unless the
  user requested it. Never commit, push, deploy, or install/replace binaries
  unless explicitly requested.

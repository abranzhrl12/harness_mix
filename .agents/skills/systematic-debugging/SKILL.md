---
name: systematic-debugging
description: Investigate a bug, failed test, unexpected behavior, or performance problem before proposing a fix.
---

# Systematic debugging

**Find the cause before changing code.** Complete the investigation stage before
choosing a fix; do not treat a symptom or plausible guess as the cause.

1. **Reproduce and inspect.** Read the full error, reproduce the behavior, and
   check recent changes, inputs, configuration, and environment. If it is not
   reproducible, gather more evidence rather than guessing.
2. **Trace the path.** Compare with a working example and follow the relevant
   data through its dependencies. For a deep call chain, use
   [root-cause tracing](root-cause-tracing.md). For multi-component systems,
   load [boundary investigation](references/multi-component-investigation.md).
3. **Test one hypothesis.** State why one cause fits the evidence, then run the
   smallest diagnostic or test that could disprove it. Change one thing at a
   time.
4. **Fix and verify.** Once the cause is established, use
   [test-driven-development](../test-driven-development/SKILL.md) for a code
   change. Keep the fix narrow and use fresh checks before claiming success;
   see [verification-before-completion](../verification-before-completion/SKILL.md).

If a hypothesis fails, return to the evidence and form a new one. After three
failed fixes, stop and reconsider the design; discuss the architectural approach
before trying another. If the investigation instead shows an environmental or
external cause, document what was checked and the remaining evidence needed. Do
not use instrumentation that exposes credentials or data outside the authorized
scope.

For waiting or timing-dependent behavior, read
[condition-based waiting](condition-based-waiting.md). After finding a valid
cause, consider [defense in depth](defense-in-depth.md) only if a second guard
addresses a real boundary.

---
name: test-driven-development
description: Implement behavior changes by observing a meaningful failing test before production code, then make the smallest change that passes.
---

# Test-driven development

For behavior changes, follow **RED → GREEN → REFACTOR**. The first test must
fail for the missing behavior; without that evidence it may not protect the
change.

1. **RED:** Write one focused test for observable behavior using a real
   component where practical. Name the regression it catches and derive the
   expected result independently. Run it and confirm the failure is the
   expected assertion, not a typo, compile error, or environment failure.
2. **GREEN:** Make the smallest production change that passes. Do not add
   adjacent features or refactor unrelated code.
3. **REFACTOR:** Improve the implementation only after the test passes; rerun
   the focused test and other required checks.

If the test passes before implementation, redesign it to expose the missing
behavior. If it fails for setup or compilation, fix that first; it is not a
valid RED result. Do not keep a production implementation drafted before the
failing test as a template. Configuration-only, generated, documentation-only, and
throwaway changes may not have meaningful runtime tests; use the project's
required validation instead of inventing one. Follow the repository's allowed
test scope, and report broader checks as pending when they are not permitted or
practical.

Read [writing-good-tests](writing-good-tests.md) when choosing assertions,
fixtures, or mocks. It covers test quality and mutation checks. For the simple
RED/GREEN cycle, see [a minimal example](references/red-green-example.md).
Before claiming completion, use fresh evidence from
[verification-before-completion](../verification-before-completion/SKILL.md).

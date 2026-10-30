---
name: verification-before-completion
description: Require fresh evidence before claiming work is complete, fixed, or passing.
---

# Verification before completion

**Evidence before claims.** Before saying a change works or a task is complete,
identify the check that proves that specific claim, run it on the current edit,
read its full relevant output, and report only what it establishes.

| Claim | Evidence needed |
| --- | --- |
| Test passes | The named test command and its failure count |
| Build succeeds | Successful build output; lint or syntax checks are not a build |
| Behavior is fixed | Reproduce the original behavior after the change |
| Requirements are met | Check each requirement and record gaps |
| Entire suite passes | The full permitted suite, not a focused test |

Distinguish `validated` checks from `pending` checks. A focused test does not
prove a full suite; a syntax check does not prove compilation; a source change
does not prove runtime behavior. Do not claim more than the evidence shows.

For permission boundaries and operational failures in Harness tasks, use the
[shared operating rules](../harness-workflow/references/operating-rules.md).

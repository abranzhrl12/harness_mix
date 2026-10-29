# Superpowers integration in Harness

## What is installed

The embedded catalog contains three skills from obra/superpowers:
`systematic-debugging`, `test-driven-development`, and
`verification-before-completion`. Each retains its upstream MIT LICENSE and
support files. `harness-workflow` is the local integration layer, not a claim
that the complete Superpowers plugin is installed.

| Skill | Contribution | Integration decision |
| --- | --- | --- |
| systematic-debugging | Reproduce, investigate cause, compare working patterns, test one hypothesis | Gather evidence with Harness; read browser evidence for web failures; do not run upstream shell examples automatically |
| test-driven-development | Observe a meaningful failing test before implementation, then make it pass | Use real behavior and existing test infrastructure; preserve project test-scope restrictions |
| verification-before-completion | Require fresh evidence for success claims | Distinguish focused tests, compilation, browser/runtime checks and unrun checks |

## Friction resolved by the integration layer

Upstream debugging text uses `superpowers:...` references. In this distribution,
resolve those names to sibling SKILL.md files, not a missing plugin tool.

Upstream TDD asks for the project's suite. Some projects restrict execution to
a crate, package or use case. Follow the applicable project policy, run the
permitted required checks, and state exactly what was verified. Never describe
a bounded check as global coverage or silently omit observed failures.

Examples use Bash, direct filesystem tools and diagnostic scripts. Harness is
the primary operational channel here. Translate the technique to available
tools and the target shell; examples neither grant permissions nor override
the failure protocol. Add diagnostic instrumentation only within an authorized
edit scope and avoid exposing credentials in evidence.

Installing skills is separate from activating them. A compatible agent chooses
skills from descriptions and the routing links in harness-read/harness-edit.
MCP itself does not execute Markdown instructions. Client discovery and reload
behavior remain client-specific.

The router and operational guides link to one another for discoverability.
Read each needed skill once per task; returning to a link must not restart the
workflow. Neither skill discovery nor graph indexing is required again at every
stage. Source freshness and executed verification evidence still need to match
the actual edited revision.

Project initialization, workspace initialization and skill synchronization have
different side effects. Project init installs skills, client entries and an
index. Workspace init writes configuration and documentation only. Skills sync
installs the catalog independently. Single- and multiproject startup perform
opt-in sync; adding a project also respects its existing preference. Workspace
manifests reload between MCP calls, retaining the last valid configuration on
errors. These mechanisms do not activate skills inside the client automatically.
Confirm roots and reload status in bootstrap instead of assuming a JSON edit
was accepted. Existing operations retain their original routing snapshot.

## Distribution and maintenance

`agent-harness init` and `agent-harness skills sync` install this routing skill
with the existing catalog. Synchronization preserves modified files and reports
conflicts; it does not overwrite a user's custom instructions. Workspace init
creates configuration and a guide; it does not install every project's skills.

Keep upstream files and notices intact. Put Harness-specific adaptations here,
so updates can be compared against upstream without mixing local policy into
the imported sources. Do not silently install additional Superpowers skills.

## Check the workflow in use

- Read-only symbol question: harness-read, without a TDD or approval ceremony.
- Backend behavior change: inspect tests, observe RED, edit, observe GREEN,
  verify the required scope and report its limits.
- Frontend failure caused by a backend response: identify both project aliases,
  collect browser/network evidence, trace the boundary and confirm the cause;
  do not treat separate graph search results as confirmed cross-project edges.
- Tool transport failure: diagnose with harness_doctor and report the blocker;
  do not continue equivalent operations through shell as a debugging shortcut.
- Documentation-only edit: review the document; no synthetic runtime tests.

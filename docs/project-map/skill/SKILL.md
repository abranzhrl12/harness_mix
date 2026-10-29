---
name: project-map
description: Discover or refresh a repository map for onboarding and code navigation, with source evidence and optional diagrams or PDF. Use when a user asks how a large project is organized.
---

# Project map

Discover architecture from the repository; the user need not know its module inventory.

## Use an existing map first

For ordinary code work, consult only the relevant section of the repository's PROJECT_MAP.md and then verify the selected source paths or symbols. Do not regenerate the map, load every module, or render a PDF just to answer a local question. Expand or refresh only the portion whose evidence is missing or affected by changes.

## Evidence first

Follow the repository access instructions. With Harness, bootstrap, verify the root, reuse the session and load the local reading guide. Batch independent reads. Do not create or refresh an index merely to generate a map.

Start with workspace manifests, entry points, module facades and existing architecture documents. Expand large repositories one subsystem at a time. Keep generated, vendored and third-party trees out of the main diagram unless relevant.

Use harness_explore for relationships when a compatible index exists; retain coverage, freshness and omission notices. Read known symbols directly. Without a compatible index, use manifests and source and state the reduced coverage. Missing bounded edges never prove absence.

## Deliver a navigation aid

Prefer a concise PROJECT_MAP.md with relative links to detail under docs/project-map/. Include:
- Modules, responsibilities, entry points and source links.
- A diagram with one explicit edge meaning. Package dependencies and runtime calls are different views.
- Where to start for common changes and which checks to consider. Separate suggested commands from executed verification.
- Verified facts, documented intent, inferences and unknowns.
- Observation date, source hashes or revision, working-tree scope and refresh instructions.

Ask only about consequential intent that code cannot establish. Do not make the user describe the architecture. Keep human annotations distinct from generated content and review existing files before replacement.

## Output and implementation

Maintain the map as Markdown and JSON, with Mermaid for its diagram. This project uses Rust; any future map automation must fit its native Rust implementation and Cargo workflow. Document export is a separate future capability, not a dependency of repository mapping.

## Refresh

Compare recorded hashes with current sources and revisit affected claims. Update links and diagrams together. Preserve manual notes. State coverage and pending checks. Do not claim exhaustive architecture or improved LLM performance without measurement.

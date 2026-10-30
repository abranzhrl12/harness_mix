---
name: harness-word
description: Create software documentation as a Word document using Harness, including selective code snapshots. Use when generating a DOCX manual, technical guide, or software report.
---

# Word documentation

Use the bundled [documentation](../documentation/SKILL.md) skill to plan and
write technical content for the intended audience. Its guidance complements
the Word-specific generation and verification steps below.

Discover the live Harness tools. Bootstrap if there is no valid session; confirm
the workspace root and reuse `data.session`. Read the shared
[operating rules](../harness-workflow/references/operating-rules.md). Use the
live `harness_word` schema, not a guessed or older contract. If the tool is not
available or reports `UNSUPPORTED`, report that limitation; do not claim that
the source implementation is installed or operational.

For a Harness-enabled project, use Harness for source investigation and DOCX
creation as well as verification: follow `harness-read` for searches and
symbols, then call `harness_word` and, when available, `harness_word_preview`.
Reuse the existing session instead of asking the user to repeat this choice.
Explicit user instructions and the active project's scope take precedence.

Choose content and detail for the intended audience. Ground technical claims
in source or other available evidence, and mark unknowns. Structure a manual
with clear headings, concise steps and tables where useful. Use `code` blocks
for selectable examples; use `code_image` only for short, illustrative snippets
where a visual adds value. Include the same code as text when readers must copy
it. Add a nearby text description: the current renderer accepts `alt` but does
not persist it as image alternative text.

## Diagrams for business and technical documentation

Use D2 for process flows and architecture when a diagram helps the reader. Use
sequence diagrams when the ordering of interactions matters. First investigate
the actual system through Harness; derive nodes, relationships, decisions and
exceptions from evidence. Mark unresolved behavior as unknown. Do not invent
business rules, actors, API calls, or database operations. An illustrative
example must be explicitly labelled as such.

Call `harness_diagram` with nodes and edges using its live schema. It renders the
structured graph with the D2 executable installed on the server host. Use PNG
for Word. Take the returned image data and pass it to `harness_word` in an
`image` block with `mime_type` set to `image/png`; add a nearby caption and
textual description. The tool does not accept arbitrary D2 source. If it is
unavailable or reports a missing renderer, report the limitation accurately.
Use a small number of meaningful steps per diagram; split wide flows into
phases so labels remain readable on A4. Review affected pages at their displayed
size with `harness_word_preview`, when available. Check labels, arrows, contrast,
margins and page breaks; report any pages that could not be visually reviewed.
For example, adapt this call to the live schema and actual content:

```json
{
  "path": "docs/guide.docx",
  "document": {
    "title": "Software guide",
    "blocks": [
      {"type": "heading", "level": 1, "text": "Example"},
      {"type": "paragraph", "text": "The following function starts the application."},
      {"type": "code_image", "code": "fn main() {}", "language": "rust"},
      {"type": "code", "text": "fn main() {}", "language": "rust"}
    ]
  },
  "session": "<bootstrap session>"
}
```

The current tool creates a new workspace-relative `.docx` and rejects existing
destinations; use `createParents` when needed and supported. It does not edit an
existing document. Inspect the response's `structuredContent`, diagnostics and
output path; do not claim creation on failure. Verify the generated file using
available document inspection or rendering, and distinguish structural checks
from visual checks. Report visual review as pending when unavailable.

When available, call `harness_word_preview` with the workspace-relative DOCX
path, a 1-based `page`, and the bootstrap session. Inspect the returned image
for readable code, margins, clipping and empty pages. Request pages separately.
This headless renderer can differ from Microsoft Word in layout and fonts.

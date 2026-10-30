---
name: harness-word
description: Create software documentation as a Word document using Harness, including selective code snapshots. Use when generating a DOCX manual, technical guide, or software report.
---

# Word documentation

Discover the live Harness tools and call `harness_bootstrap`; confirm the
workspace root and reuse `data.session`. Read the shared
[operating rules](../harness-workflow/references/operating-rules.md). Use the
live `harness_word` schema, not a guessed or older contract. If the tool is not
available or reports `UNSUPPORTED`, report that limitation; do not claim that
the source implementation is installed or operational.

Choose content and detail for the intended audience. Ground technical claims
in source or other available evidence, and mark unknowns. Structure a manual
with clear headings, concise steps and tables where useful. Use `code` blocks
for selectable examples; use `code_image` only for short, illustrative snippets
where a visual adds value. Include the same code as text when readers must copy
it. Add a nearby text description: the current renderer accepts `alt` but does
not persist it as image alternative text.

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

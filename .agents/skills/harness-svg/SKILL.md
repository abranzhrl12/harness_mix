---
name: harness-svg
description: Inspect or preview existing static SVG files with Harness MCP. Not for generating raster images or editing SVG code directly.
---

# SVG Inspection and Preview

Run `harness_bootstrap` if you lack an active session and check the `harness_svg`
schema. The advertised server supports static SVGs up to 256 KiB; never assume
script execution or external resource loading.

- Use `mode=inspect` for XML structure, dimensions, and text diagnostics.
- Use `mode=preview` directly when you need to evaluate visual appearance. Do
  not run both modes routinely.
- Pass `path` and `session`; include `externalScope` only when valid
  authorization already exists. Never escalate permissions after a rejection.
- Structural inspection alone does not prove visual quality. Examine the
  rendered preview image before making visual claims; if no preview is
  available, state that limitation.
- To edit the SVG XML, use an authorized editing tool (`harness_replace` /
  `harness_write`) with current source context and verify afterward.
  `harness_svg` itself is read-only and never writes files.
- If the format, file size, or rendering features are unsupported, report the
  limitation. On runtime or transport failure, diagnose with `harness_doctor`
  without equivalent retries or shell bypasses.

Conclude with the relevant findings; avoid dumping raw XML when structural
summary or preview is sufficient.

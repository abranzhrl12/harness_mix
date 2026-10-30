# HTML report guide

Create one self-contained HTML file in a safe temporary directory. Keep it
offline: use inline CSS and SVG diagrams, with no remote scripts, CDNs, or
project code. If a preview tool is available, use it; otherwise provide the file
path without assuming an application or shell command can open it.

## Structure

- Header: repository name, date, and a small legend for modules, seams, and
  dependency leakage.
- One `<article>` per candidate: title, strength, involved files, observed
  friction, proposed change, expected gains, and risks.
- Before/after diagrams side by side. Use inline SVG for call flow, layer
  collapse, or module surface/implementation comparisons; label both states.
- End with the best-supported recommendation and why it ranks first.

Keep the prose concise. Every candidate needs source evidence; do not propose a
generic cleanup without tying it to concrete friction. Use the vocabulary in
[codebase-design](../codebase-design/SKILL.md) when it helps explain the design.

## Review checks

- Diagrams match the described dependency flow and do not invent call edges.
- File paths and recommendations are traceable to the reviewed code.
- Note an ADR conflict only when the observed friction warrants reopening it.
- Do not implement an unselected proposal. Once the report is ready, ask which
  candidate the user wants to explore.

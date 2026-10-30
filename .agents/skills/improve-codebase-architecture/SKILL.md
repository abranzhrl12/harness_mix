---
name: improve-codebase-architecture
description: Review a codebase for deepening opportunities, present candidates, then explore a user-selected option.
disable-model-invocation: true
---

# Improve Codebase Architecture

Use only when the user explicitly requests an architecture review. Look for
specific deepening opportunities: changes that make a module easier to use and
change by concentrating policy behind a clear interface.

## Explore

Use the scope the user named. Otherwise, inspect recent changes first and widen
only when they reveal no useful area. Read the project glossary and relevant
ADRs before proposing a change. If available, read
[codebase-design](../codebase-design/SKILL.md) for the vocabulary and design
principles; use ordinary language where it communicates more clearly.

Look for a concrete source of friction: work spread across files, duplicated
policy, leaked details, a shallow interface, or behavior that is hard to test
through its real entrypoint. Trace evidence before making a recommendation. A
separate reviewer is optional and only appropriate when delegation is
available, authorized, and useful; continue the review yourself otherwise.

## Report

Write a self-contained HTML report in a safe temporary location. Describe a
small set of candidates with the involved files, observed friction, proposed
change, expected benefit, risks, and a before/after diagram when it clarifies
the design. Use the local [HTML report guide](HTML-REPORT.md) when helpful; adapt
its examples to the available file and preview tools. Do not assume a particular
skill invocation tool, shell command, browser, or network connection.

Recommend the best-supported candidate and ask which option the user wants to
explore. Do not implement an unselected proposal.

## Explore a selected candidate

Clarify constraints through discussion, then read
[grilling](../grilling/SKILL.md) if a structured decision interview helps.
Consult [domain-modeling](../domain-modeling/SKILL.md) when the selected change
affects project terminology, a glossary, or an ADR. For design alternatives,
read [codebase-design](../codebase-design/SKILL.md) and its design-it-twice
reference. Use only the tools and permissions available in the current task.

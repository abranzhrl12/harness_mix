# Multi-component investigation

Load this reference when a failure crosses boundaries such as client → MCP
server → application → database, or workflow → build → signing.

At each boundary, compare the input and output, and check how configuration and
state propagate. Capture enough evidence to locate the failing boundary before
changing it. Prefer existing logs, traces, or targeted probes; if temporary
instrumentation is necessary and authorized, remove it after diagnosis and
never log credentials or private payloads.

Once the failing boundary is known, trace backward from its bad output to the
first point where the input or state diverges. Then inspect that component and
its immediate dependencies. Do not add retries, fallbacks, or validation at
other layers until the cause is established.

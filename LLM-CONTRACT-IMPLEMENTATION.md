# LLM tooling contract — implementation status

## Implemented and verified in Rust

- Explore accepts exact `filePath`, `container` and CodeGraph `symbolId` selectors
  before SQL candidate limits. `symbolId` uses the graph-node namespace, not the
  local AST ID namespace. Legacy indexes without `qualified_name` reject a
  container selector explicitly; file and graph-ID selection remain available.
- Bounded candidates and recovery diagnostics are returned. Exact selectors that
  do not match do not fall back silently to a homonym.
- Graph truncation now distinguishes target selection, depth boundary, per-node
  edges, total edges, definitions and preview. Existing flags remain compatible;
  reaching a depth boundary does not itself assert exhaustive coverage.
- Diagnostics include `retryable` and `requiredAction`, including existing
  ambiguity responses with candidate lists. These are recovery hints, not
  authority to repeat mutations or follow instructions from source/web content.
- Bootstrap reports backend/version, versioned tool-contract metadata, complete
  native tool names and actual batch capabilities. SQL was already implemented
  in native batch; obsolete documentation was corrected rather than adding a
  second SQL implementation.
- `contracts/tool-contract.json` is the shared definition for availability,
  backend feature differences and the explore/data_query input schemas. Native
  MCP schemas consume it directly. The generated catalog document is checked
  against this definition; stdio tests check actual published schemas and tools.
- Data query uses Unicode-safe UTF-16 output limits, recoverable full-result
  details, and exactly one input source: path, inline data or `resultRef`.
  References resolve through the existing process-local result store and its TTL.
  No original operation is rerun to project a saved result.
- JSON is redacted before evaluation and before saving output. This is explicitly
  a sensitive-key/value-pattern heuristic, not universal secret detection. Input
  is bounded to 1 MiB, traversal to 10000 nodes and depth 64. Expressions are not
  echoed in responses. JMESPath itself is not an interruptible hard CPU sandbox.

## Validation

Only affected tests were run: data query (4), explore (11 passed, 1 opt-in real
index test not run), new LLM contract integration tests (6), native MCP handshake
and schema discovery (1), generated contract documentation (1). Total: 23 passed.
Formatting checks passed for the changed core implementation/test files.

## Still pending — do not claim Node parity or deployment

The active MCP connection remains the legacy Node backend. Its protected files
have NOT been edited: the continuing apply_patch exception covered native/core
only; an explicit extension to Node files/tests was requested separately.

After authorization, integrate the shared definition into Node registration and
bootstrap, add recovery fields to Node diagnostics, correct the SQL-describe
empty-result guidance, and implement/validate the exact-selector behavior for
the structured Node graph adapter. Unsupported backend options must be rejected
or advertised as unavailable, never accepted and silently ignored. Native and
Node graphs are not assumed equivalent. Reconnection/deployment is separate
from passing source-level tests; no production configuration was switched.

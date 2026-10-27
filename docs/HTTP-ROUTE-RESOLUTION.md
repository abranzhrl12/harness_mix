# Static frontend HTTP links

The native indexer connects literal frontend HTTP requests to indexed C# controller actions. It emits ordinary `calls` edges with `transport: "http"`, so existing outbound exploration and inbound impact queries include the network boundary without a schema migration.

## Supported subset

- C# classes directly inheriting from `Controller`, `ControllerBase`, or `ApiController`.
- Public instance actions, excluding `NonAction`.
- Conventional `/ControllerWithoutSuffix/Action` routes when no attribute template applies.
- Literal class/action `Route` attributes and `HttpGet`, `HttpPost`, `HttpPut`, `HttpPatch`, `HttpDelete`, `HttpHead`, and `HttpOptions` attributes, including their `Attribute` suffix forms. A class prefix combines with an action template; a bare HTTP attribute uses the class template when present.
- JS/TS/JSX/TSX `fetch`, axios verb methods, and jQuery `get`, `post`, `getJSON`, and `ajax`. Inline Vue scripts use the same extraction.
- Literal strings and template strings without substitutions. Known HTTP methods distinguish otherwise identical paths.

Matching lowercases paths, trims surrounding slashes, and strips query strings and fragments. External URLs, dynamic expressions, route parameters, token replacement, and path traversal segments remain unresolved. Ambiguous destinations remain pending instead of choosing an arbitrary action.

## Publication and evidence

Backend route metadata is persisted in method decorators using the reserved `harness:http:` prefix. Pending request references carry their original URL and method as serialized JSON. Resolved edges retain that reference, the request location, and source/target hashes.

A C# publication recovers existing HTTP edges into pending references and resolves them against the updated catalog inside the same transaction. This handles route changes, deletions, and newly ambiguous destinations even when method identity stays unchanged. Source and target bytes must match their indexed hashes before a link is published; they are checked again before commit.

The catalog and retries use existing bounded publication limits. This is a literal route heuristic, not a model of ASP.NET startup routing, middleware, areas, custom conventions, inherited controllers, or JavaScript data flow. It assumes an application's local URL paths refer to the indexed backend, and recognizes HTTP clients by their conventional names (`fetch`, `axios`, `$`) without validating their bindings; locally shadowed identifiers can produce heuristic false positives.

## Deferred work

Route-path selectors and route display in `harness_symbols`, Razor/HTML request extraction, Angular-specific client detection, parameterized routes, and Node/Python backends are outside this minimal delivery. Existing indexes acquire the new route metadata when their files are explicitly re-extracted using the supported index maintenance workflow; building the executable alone does not update an index.

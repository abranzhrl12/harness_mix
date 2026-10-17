# CodeGraph native extraction subset

Upstream: https://github.com/colbymchenry/codegraph
Tag: v1.6.2
Commit: 6560052a6f856855d3f71eee838fd66ccfa4285d
License: MIT, preserved in LICENSE.

The buffers, C#, Rust, TS/JS walkers, IDs, text, docstring and stack helpers
are copied unchanged from codegraph-kernel/src at that revision. This package
replaces ONLY the N-API entry point and grammar registry with an ordinary Rust
library. There is no Node/N-API dependency or build script. Other languages and
the upstream TypeScript preprocessing/resolution/synthesis layers are not
included. Parser deferrals remain errors; there is no hidden Node fallback.

The custom lib.rs also contains harness-only AST helpers for a conservative
named-import resolver: parsing named bindings, rejecting potentially shadowed
calls and recognizing explicit named function exports. These helpers are NOT
upstream walkers or a claim of resolver parity. The copied walker files remain
unchanged. The harness resolution/publication policy lives in src/adapters.

The package is excluded from workspace membership: upstream unit tests include
walkers outside this subset. Harness integration tests exercise the selected
walkers through the headless entry point.

schema.sql is also copied unchanged from upstream src/db/schema.sql at this
revision. It is used only by writer integration tests to create provider-shaped
fixtures; the production writer never creates or migrates a schema.

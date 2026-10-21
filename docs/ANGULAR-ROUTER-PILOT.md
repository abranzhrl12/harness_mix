# Angular Router subset

This Rust adaptation is informed by `colbymchenry/codegraph` v1.6.2 at commit
`6560052a6f856855d3f71eee838fd66ccfa4285d`, particularly
[`angular-router.ts`](https://github.com/colbymchenry/codegraph/blob/6560052a6f856855d3f71eee838fd66ccfa4285d/src/resolution/frameworks/angular-router.ts)
and [`angular-template-synthesizer.ts`](https://github.com/colbymchenry/codegraph/blob/6560052a6f856855d3f71eee838fd66ccfa4285d/src/resolution/angular-template-synthesizer.ts).
The upstream project declares MIT; its attribution and license notice are retained
in `vendor/codegraph-native/LICENSE`.

The implemented slice extracts only registered, literal TypeScript route tables
using `RouterModule.forRoot`, `RouterModule.forChild`, `provideRouter`, and
namespace-imported `RouterModule` calls. It requires an imported `Route` or
`Routes` type for named array variables, a local non-exported `const` binding,
exactly one binding reference at a recognized registration in the same file,
and static route-object/path structure. Inline literal arrays are also accepted.
Exported route tables registered from another file remain unsupported.
Nested literal `children`
are flattened. Redirects, matcher routes, `loadChildren`, dynamic paths,
spreads, computed properties, and dynamic children are omitted.

Root registrations produce route declaration candidates with `/`-prefixed paths.
`forChild` nodes use an `@forChild` marker and explicitly state that their mount
is unresolved, so their paths must not be read as effective application URLs.
Routes bind only through unique, current named runtime class imports. Importer
and target file hashes, named exports, and the exact source reference location
are rechecked during publication. A missing, stale, type-only, ambiguous, or
forged reference remains pending; there is no same-name global fallback.

This does not implement lazy route mounting, `loadComponent`, Angular template
component/event synthesis, `routerLink`, or imperative navigation. Extracted
route declarations do not certify Angular runtime activation or type checking.

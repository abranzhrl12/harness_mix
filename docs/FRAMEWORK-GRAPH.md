# Vue and Angular native graph coverage

Native graph extraction accepts `.vue`, `.html` and `.htm` in addition to the
existing languages. Angular source remains TypeScript; HTML is not classified as
Angular merely because of its extension. No JavaScript runtime, package install,
framework compiler or project code execution is required by these extractors.

## Vue

The SFC adapter reuses the bounded Vue structural parser and runs the native
JavaScript/TypeScript kernel over inline script projections. Masking outside a
script preserves line endings and UTF-16 columns in the original SFC, including
non-BMP Unicode. The graph includes the SFC component, blocks, script declarations,
template elements and style structure. Relationships from markup and styles are
structural observations, not proofs of runtime bindings or selector matches.

External block sources and unsupported preprocessors are not read or executed.
Coverage notices identify skipped content. Unproven cross-file Vue references
remain pending instead of resolving to an unrelated global symbol with the same
name. Native publication can resolve named-value-import calls from each inline
script to indexed JS/TS function exports, including constant arrow/function
expressions and named reexports. It proves lexical binding per script block,
preserving aliases, Unicode and original UTF-16 coordinates. Default/namespace,
external-script and template-expression bindings remain unsupported.
An additional bounded proof follows a direct zero-argument named composable call
into a constant object receiver or destructured method binding. It links supported
method calls to the exact native function declaration (including its original
coordinates), not to a same-named function elsewhere. The composable must return
one unconditional literal object mapping the requested property to a directly
declared local function or constant arrow/function expression. Dynamic returns,
spreads, accessors, asynchronous composables, escaping/mutated receivers and
unsupported alias flows remain unresolved. This is not general data-flow or Vue
reactivity analysis.
Initialized constant state declarations can coexist with those local helpers;
their values are not inferred. Returned helpers may be async even though the
composable itself must be synchronous. Control-flow/side-effect statements before
the final return, mutable state declarations, exported object receivers and direct
`eval` remain outside this proof. Calls with arguments, deep property chains and
methods created by factories are also unsupported by the returned-method subset.
Vue compiler transforms, template type checking, CSS preprocessing and
runtime component resolution are not implemented.

## Angular and HTML

The TypeScript graph is preserved and enriched with static Angular component
metadata recognized through imports from `@angular/core`. Literal inline templates
can contribute structural graph facts. Dynamic or unsupported metadata remains
explicitly outside the reported coverage. Decorator recognition is not dependency
injection analysis, compilation or application execution.
Imported `Directive`, `Injectable` (including no-argument decorators), `Pipe` and
`NgModule` also contribute role markers linked to unambiguous native class nodes.
Type-only imports do not prove runtime Angular decorators.

An independent lexical DI pass records imported class-token dependencies in
Angular constructor parameters, explicit `@Inject(Token)` overrides and supported
direct `inject(Token)` sites. A class-token link proves the referenced declaration,
not which provider/instance an injector will select. Dynamic/string/local tokens,
type-only imports, ambiguous exports and unsupported injection contexts stay
unresolved; an explicit unproved override never falls back to the parameter type.
Providers, factories, scopes, optionality and injected-instance method dispatch
are not evaluated at runtime.

Direct static `providers` arrays in imported Angular `Component`, `Directive`,
and `NgModule` decorators can also contribute registration observations for
`useClass` and `useExisting`. A separate native registration node references its
imported class token and implementation, with explicit relationship metadata.
These are declarations of a registration, not a proof that a particular consumer
receives that implementation. Unsupported/dynamic entries, ambiguous arrays,
factory/value/multi providers, local or string tokens and injector precedence
remain outside this subset. The regular import/export resolver and source-hash
checks validate both class declarations; no project code is executed.

The [Angular Router subset](ANGULAR-ROUTER-PILOT.md) adds literal route
declarations registered through `forRoot`, `forChild`, or `provideRouter`.
Nested literal children and named runtime component imports are supported;
`forChild` mounting remains unresolved. These are declaration observations,
not proof of runtime activation. Lazy mounting, template events, `routerLink`
and imperative navigation remain outside this implementation.

HTML extraction records bounded template structure without assuming Angular
context. A literal `templateUrl` creates an explicit pending file reference; it
can resolve only to a unique indexed HTML file whose bytes still match its indexed
hash. Source component bytes must also match their indexed hash. Changes and
deletions recheck the association within the existing publication transaction.
Unindexed, unsafe, stale or ambiguous targets stay pending.

Bindings, directives, interpolation and control-flow markers are structural
observations. Their expressions, scope, types, pipes, selectors and event handlers
are not resolved by the Angular compiler. Escaped/dynamic template literals may
be deferred rather than assigned inaccurate original-source coordinates.

## Publication and architecture

Extraction is pure: source text and `OperationContext` enter adapters; graph data
and coverage leave them. Filesystem access, SQLite and freshness checks remain in
the native publication adapter, behind the existing application ports. Helpers
share coordinate/projection policy without introducing infrastructure dependencies
in the domain. Tests live outside production source files.
Coordinates use a shared sparse line/UTF-16 index, rather than rescanning the source
for every structural node. Source sizes, node counts and control-flow scanning are
bounded; cancellation is checked between extraction steps.

Synthetic node IDs reserve `native:framework:`. Publication replaces these nodes
on edits/deletion while preserving provider-owned component/route synthesis. The
existing refusal of unsupported provider relationships is retained. Coverage is
returned as `extractionCoverage` in previews and changed-file publication results;
it must not be interpreted as whole-project or compiler-level verification.

Semantic facts are memoized per source within one publication. A bounded runtime
export memo (128 entries / 2 MiB) avoids repeating the same export walk, including
negative answers. It is discarded with the transaction's source/configuration
snapshot. All observed source hashes are rechecked before commit. Edits, deletions,
new modules and configuration changes invalidate framework engine links and retry
pending references under the current indexed snapshot. Global invalidation remains
deliberately conservative; this is not a latency benchmark or a promise of
whole-project semantic analysis. General returned-value propagation, Vue runtime
reactivity and Angular runtime provider selection remain outside scope.

The inventory and watcher use the same extension dispatch. No project index is
created or rebuilt merely by reading or extracting a source file.

## Performance measurements

The opt-in [framework benchmark](BENCHMARK-FRAMEWORK.md) separates extraction,
publication and incremental target changes on deterministic temporary fixtures.
Its PowerShell wrapper reports observed process peak working set separately from
operation timings. Synthetic source/reference sizes do not represent large
workspaces, and passing correctness tests does not establish a latency guarantee.

## Language references

- [Vue SFC specification](https://vuejs.org/api/sfc-spec)
- [Vue script setup](https://vuejs.org/api/sfc-script-setup.html)
- [Angular component metadata](https://angular.dev/api/core/Component)
- [Angular templates](https://angular.dev/guide/templates)
- [Angular injection contexts](https://angular.dev/api/core/inject)
- [Vue composables](https://vuejs.org/guide/reusability/composables)

The native implementation deliberately covers a bounded subset of these formats,
not the complete behavior of their official compilers.

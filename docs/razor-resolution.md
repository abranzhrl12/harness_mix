# Razor navigation

The native index connects Razor member blocks and render expressions to C#
methods without starting Roslyn or an ASP.NET application. `harness_explore`
uses these edges normally; callers can specify `filePath` and `symbol` without
duplicating `query`.

## Typed receivers

The C# AST tracks lexical scopes for fields, properties, parameters, explicitly
typed locals and `var` initialized with `new`. Razor `@inject`, `@model` and
`@using` contribute type information. Calls in `@functions`, `@code`, inline
expressions, explicit `@(...)` expressions and `@{...}` render blocks retain
their original UTF-16 locations. A public instance method is linked only when
both its receiver type and method are unique in the indexed declarations.

`dynamic`, unknown aliases, lambdas, generic types, overloaded methods and
unproven receiver chains remain unresolved. Inherited members require additional
semantic binding; this is not a full C# compiler.

## MVC destinations

Literal `Url.Action`, `Html.ActionLink`, and `asp-action` / `asp-controller` /
`asp-area` attributes resolve through conventional `Views`, `Controllers` and
`Areas` directories. App roots and areas remain separate. Controller and action
names are case-insensitive. Literal `ActionName` and `Area` attributes participate
in selection; `NonAction`, `NonController`, ambiguous actions, generic, static
and abstract candidates are excluded. Literal route templates do not prevent
controller/action selection.

Dynamic routing, named helper overloads, inherited actions and arbitrary `href`
URLs are not inferred. Shared views require an explicit controller. Custom
endpoint conventions, dependency injection registrations and runtime routing
transformers are outside this static subset.

## Legacy view composition

`Html.Partial` and `Html.RenderPartial` resolve a literal view name to the
partial file; `Layout = "..."` resolves the layout. Both use the conventions
of the referencing view: `Views/{controller}` and then `Views/Shared` of the
same area, or an application-relative `~/` path. Two matching files stay
unresolved instead of being guessed. These calls are also detected inside
inline C# regions (`@{ ... }`, `@(...)`) and inside `@code`/`@functions`
blocks, where a call written inside a C# string is not evidence.

`Html.Action` and `Html.RenderAction` reuse the literal route subset above.
Named arguments, `_ViewStart.cshtml` inheritance and `RenderSection` are not
inferred. The complete subset and its limits are described in
[razor-legacy-mvc.md](razor-legacy-mvc.md).

## Cost and incremental updates

Receiver facts are parsed once per observed view per publication. Positive and
negative method targets share a bounded cache. Render expressions use a shared
projection rather than one parser invocation per expression; source mapping uses
binary searches. Receiver bindings use a balanced interval index, and column
mapping advances a single UTF-16 cursor, including long lines with many calls.
MVC lookups share a bounded controller snapshot: the first
controller lookup reads its class and methods, and subsequent actions reuse the
result. Compiled regular expressions are reused.

Changed C# method/type/controller names trigger bounded rechecks. Pending calls
use the existing failed-reference tail index. Resolved edges retain their
original reference names, so additions, deletions and newly ambiguous targets
can invalidate and restore links without resynchronizing every view. Sync
results report `razorReferencesRechecked`. All caches belong to the publication,
not a global lifetime, and source observations participate in the existing hash
recheck before commit. No schema migration or whole-project reindex is required
by this resolver; existing views need normal synchronization to acquire the new
references.

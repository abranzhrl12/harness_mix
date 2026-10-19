# Vue structure and extraction preview

`harness_symbols` supports Vue SFC navigation without Node or a workspace index.
The bounded native structural parser builds SFC blocks, an HTML template element
tree, CSS/SCSS rule trees and JS/TS AST declarations, imports and Vue macros.
It is a structural subset, not the Vue compiler or Sass evaluator. Unsupported
template languages and external blocks are reported rather than read implicitly.

## Outline and exact reads

```json
{"action":"outline","path":"src/Parent.vue","maxChars":16000}
```

`structure` contains block languages and flags (`setup`, `scoped`, `module`),
template parent indices, directives and ranges, and style selectors with their
static class candidates. Nested `&__element` BEM selectors retain their resolved
parent context. Associations are marked `static_class_candidate` or `unknown`;
they do not prove CSS matching, inheritance or runtime dynamic-class usage.
Omitted items and structure are recoverable through the result's detail reference.

Use an element's `name` and `symbolId` with `action:read, view:full` to get its
exact source and the whole-file hash. IDs depend on that hash and become stale
when the SFC changes. Script declaration byte ranges are remapped to the SFC.

## Combined extraction proposal

```json
{
  "action":"extract_preview",
  "path":"src/Parent.vue",
  "expectedHash":"<64-character SHA-256 from outline/read>",
  "symbolId":"<24-character template element ID>",
  "targetPath":"src/components/ProfileCard.vue",
  "componentName":"ProfileCard"
}
```

The operation checks the source hash and destination absence before and after
planning. Protected/outside-workspace destinations and existing files are refused.
It returns one patch for both files; full proposed contents and hashes are in
detail. No files or directories are created and the index is not changed.

The parent proposal adds a setup import and substitutes the selected element.
The child proposal contains that element, suggested runtime props, and complete
static rule trees used exclusively by that subtree. Original newline conventions
are retained. Shared or uncertain rules stay in the parent with explicit review
notes: dynamic classes, scoped boundaries, Sass state, CSS variables, asset URLs,
animations and selector contexts may prevent a safe style transfer.

Suggested props are lexical identifier candidates, not proven free variables.
`unresolved` calls out expressions, callback/event semantics, component imports,
loop/slot scopes, writable bindings, refs and other boundary changes. Dependent
conditional branches are refused. Generated scripts and structural syntax are
checked, but compilation and type checking remain separate.

`previewOnly:true`, `writesPerformed:false`, and `applicationSupported:false`
are always explicit. This release does not supply an atomic multi-file apply or
recovery operation. Any subsequent application must review the complete proposal,
recheck hashes and target absence, and verify the project. A truncated patch is
not a substitute for full proposed contents.

Limits: 512 KiB per SFC, 64 blocks, 2,000 elements/rules/symbols, 128 template/style
levels, and bounded selector expansion. Parsing honors operation cancellation.

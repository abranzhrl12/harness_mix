# Optional read presentation

`harness_read` keeps its existing response format by default. Two optional
boolean arguments control presentation without changing the source being read:

```json
{
  "path": "Views/Example.cshtml",
  "start": 15,
  "lines": 26,
  "compact": true,
  "lineNumbers": true
}
```

`compact: true` removes empty envelope diagnostics, empty omission metadata,
null detail references and the public request ID. Nonempty diagnostics,
omission notices and detail references remain available. Internal request
correlation is unchanged. The result retains `state`, `scope`, and `data`.

`lineNumbers: true` adds `data.numberedText`, a display-only representation
with absolute file line numbers (`15 | ...`). `data.text` remains the exact
source fragment, including its original line endings. Use `text`, not
`numberedText`, for replacements. The full file hash and range metadata remain
unchanged; line numbering does not replace hash validation before editing.

These options do not request indexing or dependency discovery. Omit them (or
set them to `false`) for the existing output contract.

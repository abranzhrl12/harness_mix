---
name: harness-xlsx
description: Inspect or read existing XLSX workbooks via Harness MCP by sheet, range, or page. Not for creating workbooks or controlling an open Excel instance.
---

# Bounded XLSX Reading

Run `harness_bootstrap` if you lack an active session and follow the advertised
`harness_xlsx` schema. Never load an entire workbook just to discover a sheet.

- If sheet names are unknown, call `harness_xlsx` with `path` and omit `sheet`
  to retrieve workbook metadata.
- If the sheet and range are known, query them directly with `path`, `sheet`,
  and `range`. For large tables, paginate using bounded `rowOffset` and
  `pageSize` values based on the response.
- Request cell formulas via `formulas` only when needed. Do not confuse formulas,
  cached values, and live recalculation: never claim to have recalculated a
  workbook using a read-only tool.
- Preserve file hashes and coverage metadata. Pass `expectedHash` when page
  consistency is required and supported; if the underlying file changes, restart
  reads for the affected range rather than mixing versions.
- Project large outputs with `harness_data_query` when supported. Never treat
  omitted cells or empty values as zeros without evidence.
- Respect path and scope boundaries; a permission rejection does not authorize
  reading the file through an alternative channel. This tool does not modify
  workbooks: if creation or editing is requested, use an appropriate authorized
  capability.

Example (replace with real values):

```json
{"path":"data.xlsx","sheet":"Sales","range":"A1:F30","session":"SESSION_ID"}
```

Report the inspected sheet/range, results, and any relevant interpretation limits.

# Shared harness contract

Generated from `contracts/tool-contract.json`. Contract version: 3.0.0.

| Tool | Backends |
| --- | --- |
| harness_batch | node, rust |
| harness_bootstrap | node, rust |
| harness_browser | node |
| harness_checkpoint | node, rust |
| harness_config | node, rust |
| harness_data_query | rust |
| harness_detail | node, rust |
| harness_diagnostics | node, rust |
| harness_doctor | node, rust |
| harness_explore | node, rust |
| harness_external_scope | node, rust |
| harness_git | rust |
| harness_git_diff | node |
| harness_git_status | node |
| harness_handover | node, rust |
| harness_impact | rust |
| harness_jobs | rust |
| harness_logs | node, rust |
| harness_move | rust |
| harness_observe | rust |
| harness_outline | node |
| harness_patch_preview | rust |
| harness_process_status | node, rust |
| harness_read | node, rust |
| harness_read_symbol | node |
| harness_regression | node |
| harness_replace | node, rust |
| harness_write | rust |
| harness_search | node, rust |
| harness_sql | rust |
| harness_sql_describe | node |
| harness_sql_query | node |
| harness_sql_search | node |
| harness_sql_status | node |
| harness_svg | node, rust |
| harness_symbols | rust |
| harness_tasks | node, rust |
| harness_verify | node, rust |
| harness_web_check | node |
| harness_xlsx | rust |
| harness_xlsx_inspect | node |
| harness_xlsx_read | node |

Schemas for explore and data_query are consumed directly by native MCP. Node integration must consume the same definitions; backend differences remain explicit.

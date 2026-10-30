# D2 diagrams through MCP

`harness_diagram` renders a PNG from a declarative graph. The server must have a trusted D2 executable on `PATH`, or `HARNESS_D2_PATH` must point to its absolute path before the MCP server starts. D2 is not bundled with Harness. Restart the MCP connection after updating the server executable or its environment. The tool is available in the `edit` and `full` profiles.

Example request:

```json
{"nodes":[{"id":"request","label":"Receive request"},{"id":"review","label":"Review"}],"edges":[{"from":"request","to":"review","label":"Submit"}],"format":"png"}
```

The result contains a PNG image and `data.imageBase64`. To insert it into a Word document, pass that value as `data_base64` in a `harness_word` image block with `mime_type` set to `image/png`. Verify the output with `harness_word_preview` where available. Labels and relationships must be supported by evidence; split wide flows into readable phases.

The API accepts at most 24 nodes and 48 edges. It generates D2 internally instead of accepting arbitrary D2 source or paths. It does not support every D2 diagram type or styling option. Renderer errors and missing dependencies are reported by the tool.

# Browser inspection

`harness_browser` connects to a Chromium browser that exposes the Chrome DevTools Protocol (CDP). It lets an MCP client inspect the rendered page, collect browser errors, and view a screenshot without adding browser-specific types to the application layer. Connect with either the direct `ws://localhost:PORT/devtools/browser/ID` endpoint or a local `http://localhost:PORT` discovery URL. Discovery requests only `/json/version` and accepts only `localhost`, `127.0.0.1`, or `[::1]`, with an explicit port and no credentials, query, or fragment. Redirects are rejected, the response is size and time limited, and the returned `webSocketDebuggerUrl` must be a loopback endpoint on the same port. Remote hosts, HTTPS, and unrelated URL paths are rejected.

The `harness-browser` crate implements the application `BrowserInspection` port and owns CDP/runtime details. Its default `BrowserService` connects lazily; `BrowserService::with_backend` accepts an injected `BrowserBackend` for alternate implementations and isolated tests. The domain and application contracts do not depend on Chromiumoxide or Tokio.

Use `connect` with `endpoint: "auto"` and `tab: "http://localhost:PORT/path"` to prepare the browser automatically. Harness discovers local CDP sessions first; if none exists, it launches an installed Chromium browser with a separate temporary profile and an ephemeral loopback debugging port. It creates and attaches a blank tab before opening the requested application, capturing initial-load exceptions. Existing tabs are preserved; if the application is already open, it attaches that tab. The separate profile may require signing in again. Without an application URL, the newly launched browser has an attached blank tab for manual navigation. No shell command or fixed debugging port is required. Explicit CDP endpoints retain connection-only behavior; `discover` never launches a browser. `BROWSER_LAUNCH` reports missing Chromium or launch failures.

For an existing explicit CDP endpoint, use the browser tool in this order:

1. `connect` with either the direct DevTools WebSocket URL or the local HTTP debugging address shown by Chrome; for HTTP, Harness discovers the WebSocket from `/json/version`. Keep the returned `connectionId`.
2. `targets` with that ID to list open pages and their `targetId` values.
3. `attach` to the page before reproducing a problem so console, exception, and network events can be collected.
4. `inspect` with a `targetId` and optional CSS `selector` to read the rendered HTML, text, element bounds, and computed styles. `maxChars` limits returned HTML.
5. `errors` to page through the captured events with `after` and `limit`.
6. `screenshot` with an optional selector to capture the page or selected element. The result is returned as MCP image content; width and height remain in structured result metadata.
7. `disconnect` when inspection is complete.

All actions require the `session` returned by `harness_bootstrap`. Every action except `connect` requires the `connectionId`; page actions also require a `targetId` except `errors`, where it is optional. A successful `attach` enables event collection for that page. Capture starts at attachment; complete historical coverage is not guaranteed. A JavaScript render loop is detectable when the framework or browser reports it; silent repeated renders may need framework instrumentation. Event timestamps record Harness receipt time in milliseconds since the Unix epoch; line and column positions retain CDP's zero-based coordinates. Pass `nextCursor` back as `after` to read the next page; `dropped` counts overwritten events in the connection's ring.

The tool inspects the DOM after JavaScript has modified it. It does not read the original source template. Bounds use viewport-relative CSS pixels; screenshots report their actual PNG pixel dimensions. Harness allows up to eight browser connections and one per Harness session; a target listing with more than 128 pages is rejected, and at most 32 pages can be attached per connection. It retains a 256-event ring. Event messages and stacks are bounded and mark truncation; HTML and text are capped at 100,000 UTF-16 units each (24,000 by default). Screenshots are PNG, capped at 4 MiB and 4096 pixels per dimension. Operations pass through a queue of 32 requests and run serially with a 20-second operation deadline; a slow operation can delay later requests. Disconnect aborts the connection's event tasks without closing the external browser. Keep the debugging endpoint local and use a dedicated browser profile.

## Build feature

The Rust executable enables the optional `browser-cdp` feature by default. To build with browser CDP support:

```sh
cargo build -p agent-harness --release --locked
```

To omit the CDP backend, build with `--no-default-features`; the `harness_browser` tool remains available in the full profile but reports `UNSUPPORTED` because no backend was compiled:

```sh
cargo build -p agent-harness --no-default-features --release --locked
```

You can also explicitly enable it when disabling defaults:

```sh
cargo build -p agent-harness --no-default-features --features browser-cdp --release --locked
```

The workspace includes the `harness-browser` crate even when the executable feature is disabled. Run its tests in isolation with `cargo test -p harness-browser`; `cargo test --workspace` includes the crate's tests as well. The feature controls whether the CDP backend is included; it does not imply that Chromium is running or reachable, and no runtime speed improvement is promised. Bootstrap reports whether the backend was compiled, not whether a browser connection was probed.

## Interactions

Use `snapshot` with `connectionId`, `targetId`, and optional `limit` (1..100, default 50) to list interactive elements. Each entry has a CSS selector, tag, heuristic role and name, visibility and disabled state. Names are approximations from aria-label, aria-labelledby, labels, title, placeholder or text, not the browser accessibility tree. `truncated` reports omitted elements or shortened names; selectors describe the current DOM and can become stale. Input values are not returned.

`click` takes `selector`; `fill` takes `selector` and `value` and supports text-like inputs, textareas and contenteditable elements, emitting bubbling input and change events. These actions modify the page. `click` invokes the DOM click method even for hidden elements, without scrolling or hit testing; it does not synthesize trusted pointer events. Native disabled elements are rejected. `wait` takes `selector`, optional `state` (`present`, `visible`, or `hidden`; default `visible`) and `timeoutMs` (1..15000, default 5000). Hidden includes an absent element. Visibility is based on layout and CSS visibility, not occlusion. Selectors are limited to 4096 bytes and fill values to 100000 bytes. All actions use the same session and target ownership as inspection.

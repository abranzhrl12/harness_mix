---
name: harness-browser
description: Diagnose and verify web applications with Harness MCP via console, network, DOM, and screenshots. Use for visual, interaction, or hydration failures; not for general web browsing.
---

# Evidence-Based Web Diagnostics

Check the current `harness_browser` schema and reuse a Harness session with a
verified workspace root. Having the tool available does not mean a browser is
connected. Do not edit code until the task authorizes a fix.

## Connect and Reproduce

1. Reuse an active `connectionId` and `targetId` from the current session when
   available. Otherwise, call `connect`:
   - **Automatic local CDP discovery and `localhost` tab selection**: You can
     omit `endpoint` or pass `"auto"` or `"localhost"` to automatically detect
     active local debugging sessions and prioritize `localhost` tabs. The
     response returns `selectedTab` and `availableTabs` so you can verify the
     selected target or switch if needed.
   - **Explicit endpoint**: You may also pass a loopback browser CDP WebSocket
     (`ws://127.0.0.1:<port>/devtools/browser/<id>`). Application URLs and
     `/devtools/page/` endpoints are not valid explicit browser WebSocket URLs;
     only loopback endpoints are allowed.
   - If no active debugging session is found, ask the user for the endpoint or
     browser setup; never guess IDs, expose public debugging ports, or alter the
     user's default browser profile.
2. Inspect `selectedTab` / `availableTabs` (or call `targets`), verify the tab
   by URL and title, and call `attach` before reproducing if not already
   attached to the right tab. Never blindly pick the first tab without checking
   its URL/title.
3. Query `errors` scoped to `targetId` and save `nextCursor`. Reproduce the
   affected route and interaction, including a page reload when the failure
   occurs on initial load or hydration. `harness_browser` does not perform
   arbitrary navigation, reloads, clicks, or script execution: ask the user to
   trigger the action or use another available, authorized capability without
   inventing parameters.

Event capture starts on attach; prior errors are not guaranteed to be present.
Never conclude there are no errors unless the issue was reproduced while capture
was active.

## Investigate Without Flooding Context

- Query `errors` with `after` set to your saved cursor to separate new events
  from historical noise. `limit` accepts `1..256`; paginate with `nextCursor` if
  the limit is reached. `dropped` indicates lost events, not a complete capture.
- Distinguish console warnings, uncaught exceptions, and network failures.
  Deduplicate repeated events when reporting while preserving the message,
  source location, sequence, and reproduction steps. Never ignore hydration
  warnings just because the screen looks visually normal.
- Use `inspect` on the relevant selector to retrieve DOM structure, bounding
  boxes, and computed styles; expand scope only if evidence is missing. Respect
  `truncated`. Use `screenshot` when visual appearance matters; a screenshot
  alone does not prove the absence of runtime errors.
- Map messages and stack traces to exact source code: read known symbols
  directly with `harness_symbols`, trace dependencies with `harness_explore`, or
  run literal searches with `harness_search` to locate messages or components.
  Clearly separate confirmed root causes from hypotheses.
- For hydration mismatches, compare server-rendered and client-rendered state:
  initial values, available options, and browser-dependent data. The inspected
  DOM reflects current live state and does not by itself prove the original SSR
  HTML. Never silence hydration warnings or disable SSR as an automatic workaround.

## Verify and Deliver

After an authorized code fix, save a cursor, repeat the reproduction with active
capture, and query new events. Verify expected behavior and DOM/screenshots when
applicable, alongside relevant code verification (`harness_verify`). A clean
build or error-free HMR update does not replace a full reload for hydration bugs.

Report which route and action were verified, whether the original error
reappeared, any other relevant failures, and any capture or reproduction
limitations. Never claim the entire application is "error-free" based on a
single tab, time window, or bounded query. Never copy credentials or personal
data from DOM nodes, URLs, or logs into your report. Disconnect only
connections created for this task once finished; never close the user's browser.
On transport, timeout, or runtime failure, stop equivalent retries, diagnose
with `harness_doctor`, and report the limitation.

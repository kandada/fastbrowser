# fastbrowser — engine capability matrix & known limitations

fastbrowser speaks to pluggable engines (WebView, CDP/Chromium, mock). Tools are
capability-gated; the table shows what each engine class supports today.

| Capability | CDP (Chromium) | WebView (iOS/Android) | Notes |
|------------|:--------------:|:---------------------:|-------|
| navigate / history / wait | ✅ | ✅ | live `page_url`/`page_title`; stale-page detection |
| extract (text/html/links/images/table/json) | ✅ | ✅ | shadow DOM + same-origin iframes pierced |
| interact (click/type/hover/scroll/…) | ✅ | ✅ | `press`/`send_keys` special keys, `hover`, `drag`, `swipe`, `click_coords` synthesise DOM events; relative `scroll` via `window.scrollBy` |
| forms / select / check / radio | ✅ | ✅ | |
| JS execute / XPath / inject CSS | ✅ | ✅ (gated) | `js_injection` capability |
| a11y tree | ✅ | fallback | `max_nodes`/`maxNodes`/`limit` |
| console logs | ✅ native + buffer | injected buffer | webview installs `window.__fbConsoleLogs` (Android on `onPageStarted`, iOS document-start) |
| storage / cookies | ✅ | ✅ (`https`) | `file://`/`data:` storage restricted by browser policy |
| tabs / windows | ✅ | ✅ | `id` accepted as `tab` alias; titles refreshed live from `document.title` |
| screenshot | ✅ | ✅ | webview uses native `wv.draw` (Android) / `takeSnapshot` (iOS) |
| save_as_pdf | ✅ | ❌ | needs CDP `Page.printToPDF` |
| dialogs accept/dismiss | ✅ | ❌ (`pending_dialog` ✅) | webview records into `__fbDialogs` and auto-dismisses |
| native HTML5 drag & drop | partial | synthetic | CDP `Input.dispatchDragEvent` not wired |
| network interception / mock / HAR | ❌ | ❌ | not implemented |

## Known limitations
- **webview dialogs** — `pending_dialog` works (dialogs are recorded into
  `window.__fbDialogs`), but `dialog_accept`/`dialog_dismiss` still require a
  native blocking delegate.
- **save_as_pdf** — CDP only; webview has no `Page.printToPDF`.
- **navigate metadata** — `get_current_url`/`get_page_title` read the live DOM;
  the `navigate` return value may still lag one frame.
- **`id` fields** — never expose the internal live sentinel (`display_id()` is used).
- **Batched tool calls** — multiple `browser_call`s in one message may run
  concurrently (faster, but racy for dependent calls); serialize when you need
  deterministic ordering.
- **drag** — events are synthesised in-page (mouse/touch); native
  `dragstart/dragover/drop` may not fire on the webview engine.

## Milestone-level status
- Network interception/mock/HAR, session/context isolation, native drag, and a
  unified "action → settle" contract are **not yet implemented** (engine-bound).

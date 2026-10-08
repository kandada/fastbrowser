# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project
adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.9] - 2026-10

- Token-efficient perception: a new compact, geometry-free snapshot text view
  (`Fastbrowser::snapshot_text()`; CLI `snapshot text` / `snapshot --format text`)
  → `[id] role "text" -> href`, where `[id]` is still an actionable snapshot ref.
  The default `snapshot` JSON is unchanged.
- `get_accessibility_tree` reports `link_density` + a `hint` on link-dense pages
  (a per-node role/name tree mostly echoes link text → prefer `extract_links` /
  `get_page_text` / `view:"text"`). Its default byte budget still caps even
  `detail:"full"` unless `max_chars:0` is passed explicitly.
- Fix: the surface runtime now enables the tokio **I/O** driver (`enable_all`),
  so the Linux AT-SPI backend (zbus) no longer panics on connect — it fails
  gracefully and is reported as a skipped provider instead of taking down the
  process/daemon.

## [0.1.8] - 2026-10

- `execute_js`: a **lone, whole-script** function/arrow expression passed as
  `script` (e.g. `async () => {…}`, `x => x + 1`) is now auto-called; scripts
  with trailing statements, or arrows inside larger expressions
  (`xs.map(x => x)`, `asyncTask()`), are left untouched. (An uncalled arrow used
  to evaluate to a function, which the host serialized to `null`.)
- `download`: sends a full desktop-browser header set
  (`Accept`/`Accept-Language`/`Sec-Fetch-*`/`sec-ch-ua*`); `Accept` stays generic
  (`*/*`) except for image URLs (which get an image-typed Accept); new `headers`
  object adds/overrides request headers (`Host`/`Content-Length` are ignored).
- `navigate`: detects high-signal anti-bot/WAF interstitial titles (e.g.
  "Just a moment", "…请求已被阻断") and reports `ok:false` + `blocked:true`;
  disable with `Config.detect_blocked_pages = false`.

## [0.1.7] - 2026-10

- Desktop/pip: `auto` no longer falls back to `mock` silently. The chain is now
  external `cdp_url` → `bundled` → **`system`** (platform default browser, or a
  well-known Chrome/Edge/Chromium install; CDP-capable only) → host `webview` →
  `mock`, and the choice is always reported (`status()` / `get_info()` carry
  `engine_requested`, `engine_used`, `degraded`, `fallback_reason`, `hint`). Set
  `allow_fallback_mock=false` to fail instead of degrading to `mock`.
- New `engine:"system"`: discover and launch a system/default Chrome/Edge/Chromium
  over CDP.
- New `Config` fields: `browser_path`, `prefer_headless`, `use_default_browser`,
  `use_user_profile`, `remote_debug_port`, `allow_fallback_mock`. Real browsers
  launch headless with an isolated temp profile by default.
- Clearer missing-browser guidance: `chromium` needs `cdp_url`; `bundled`/`system`
  hints; `auto`→`mock` explanation. The CLI prints a degradation notice to stderr.

## [0.1.6] - 2026-09

- `get_accessibility_tree` is now pruned by default: noise nodes filtered,
  node/depth/text caps, geometry omitted (it was ~80% of the payload), a single
  representation (no more `tree` + `root` duplication) and a hard char budget.
  New parameters (`max_depth`, `interesting_only`, `max_text_len`,
  `include_geometry`, `detail`, `view`, `max_chars`) opt back into the full tree.
  Engine geometry is only fetched when requested (`accessibility_tree_opts`).
- `navigate` also reports a silently-ignored load for `file://` / `ftp://`
  (previously only http(s)): landing on `about:blank` or staying on the previous
  page after waiting is no longer a false `ok: true`.

## [0.1.5] - 2026-09

- Accessibility / native surface: `surface-*` features and the `ax_*` tools
  (macOS `AXUIElement`, Linux AT-SPI, Windows UIA, Android `AccessibilityService`
  via the C ABI `FbSurfaceOps`).
- Shared selector dialect for `find_elements` / `click` / `type`: `role=` with
  `[name/exact/checked/disabled/expanded/selected/level]` filters,
  `label=` / `placeholder=` / `alt=` / `title=` / `value=` / `href=` / `ref=`,
  `>>` chaining, and Testing-Library / Playwright-MCP tool aliases.

## [0.1.4] - 2026-09

- `download` tool: fetch a URL straight to an absolute host path, byte-for-byte
  (binary-safe) — exposed to the CLI (`download <url> <path>`), the SDK, and the
  in-process routers in host shells.
- Playwright MCP / browser-use compatibility: tool-name and parameter aliases
  are resolved in the kernel (`tools::aliases`), so `browser_navigate`,
  `go_to_url`, `click_element`, `expression`→`script`, etc. work everywhere.
- `execute_js` accepts both expression-style scripts and statement blocks with a
  top-level `return` (wrapped in an IIFE).
- `search` falls back to `documentElement` when the document has no `<body>`
  (e.g. a directly-opened SVG/XML document).

## [0.1.0] - 2026-09

- Initial release.
- Engine-agnostic `BrowserEngine` trait (mock / bundled Chromium / external
  Chromium via CDP / CEF / system WebView) with `auto` fallback.
- LLM-native tool manifest (JSON-Schema params, JSON outputs).
- Interactive-element snapshot (`PageSnapshot`, a/b/c ids).
- Real-browser semantics over CDP: coordinate click with Playwright-style
  actionability, real keyboard input, event stream, downloads, OSR frame
  streaming, real PNG screenshots, cookies/storage, file upload, PDF.
- Profile isolation via CDP `BrowserContext`.
- Sync shell (CLI / C ABI) and async shell (`AsyncFastbrowser`).
- C ABI (`fastbrowser_c/`) and bindings (`bindings/`).

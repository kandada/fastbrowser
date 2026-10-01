# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project
adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

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

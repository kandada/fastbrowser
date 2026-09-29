# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project
adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

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

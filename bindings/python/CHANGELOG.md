# Changelog

All notable changes to this project are documented here.

## [Unreleased]

## [0.1.8] - 2026-10

- Version aligned with the kernel (`0.1.8`).
- **CLI**: `python -m fastbrowser <cmd>` (and the `fastbrowser` console script).
  - Ephemeral one-shots: `tools` / `fetch` / `run --steps` / `download`.
  - Stateful **daemon** (auto-started; `serve` / `daemon start|stop|status`)
    that keeps one browser alive so cookies/tabs/page persist across separate
    CLI invocations. Listens on **both** a Unix socket and a loopback TCP port;
    clients auto-detect via a discovery file (`~/.fastbrowser/daemon.json`).
  - Accessibility: `ax list` / `ax snapshot` / `ax act`.
  - `execute_js`: a bare function/arrow expression passed as `script`
    (e.g. `async () => {…}`) is now auto-called (it used to evaluate to a
    function → `null`).
- `download`: sends a full desktop-browser header set
  (`Accept`/`Accept-Language`/`Sec-Fetch-*`/`sec-ch-ua*`) and accepts a custom
  `headers` object — needed to pass hot-link/WAF-protected hosts.
- `navigate`: detects anti-bot/WAF interstitial pages (e.g. "Just a moment",
  "…请求已被阻断") and reports `ok:false` + `blocked:true` instead of success.

## [0.1.7] - 2026-10

- Version aligned with the kernel (`0.1.7`).
- `auto` never silently falls back to `mock`: it also tries a **system/default
  browser** (`engine:"system"`), and reports `engine_used` / `degraded` /
  `fallback_reason` / `hint` via `status()` / `get_info()`.

## [0.1.6] - 2026-09

- Version aligned with the kernel (`0.1.6`).
- `get_accessibility_tree` is pruned by default (no geometry, node/depth/text
  caps, single representation, char budget) with opt-in parameters for the full
  tree; `navigate` no longer reports a false `ok` for a silently-ignored
  `file://` load.

## [0.1.5] - 2026-09

- Version aligned with the kernel (`0.1.5`).
- Accessibility / native-surface and selector-dialect support from the kernel.

## [0.1.4] - 2026-09

- Version aligned with the kernel (`0.1.4`).
- New `download` tool available through `FastBrowser.tool_call("download", ...)`.
- Playwright MCP / browser-use tool-name and parameter aliases resolved by the
  kernel, so calls through the Python binding accept those names too.

## [0.1.3] - 2026-09

- Cross-platform wheels built in CI: Linux x86_64 + aarch64 (manylinux),
  Windows x64, macOS universal2 (Intel + Apple Silicon).
- Version aligned with the kernel (`0.1.3`).

## [0.1.2] - 2026-09

- Version aligned with the kernel (`0.1.2`).
- Fixed sdist build by dropping `license-files` (the nested layout caused a
  `LICENSE` name collision with the kernel).

## [0.1.0] - 2026-09

- Initial release of the Python bindings (`fastbrowser`).
- `FastBrowser` (sync) and `AsyncFastBrowser` (asyncio) wrappers over the
  fastbrowser kernel, returning Python dicts/lists.
- Screenshot helpers (`screenshot`, `screenshot_png`, `save_screenshot`).
- Event/frame callbacks dispatched on a separate thread (re-entrant safe).
- abi3 wheels (Python ≥ 3.8).

# Changelog

All notable changes to this project are documented here.

## [Unreleased]

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

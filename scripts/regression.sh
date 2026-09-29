#!/usr/bin/env bash
# Full fastbrowser regression.
#
#   bash scripts/regression.sh
#
# Runs: fmt check, English-only guard, clippy, then the test suite under the
# three feature sets that matter:
#   * default            — mock engine (lib + most integration suites)
#   * engine-webview     — the offscreen WebView host suite (MockOps)
#   * engine-cdp         — the real-Chromium suite (launches headless Chrome;
#                          SKIPs automatically when no browser is found)
#
# The CDP suite needs a Chrome/Chromium binary (set CHROME_PATH to override).
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> cargo fmt --check"
cargo fmt --check

echo "==> English-only guard"
python3 scripts/check-english.py

echo "==> clippy (all targets)"
cargo clippy --all-targets

echo "==> tests: default features"
cargo test

echo "==> tests: engine-webview"
cargo test --features engine-webview --test webview_ops_e2e

echo "==> tests: engine-cdp (fast subset; heavy real-Chromium suites are opt-in)"
cargo test --features engine-cdp

echo "    (heavy real-Chromium tests: bash scripts/cdp-full.sh)"

echo "==> fastbrowser regression OK"

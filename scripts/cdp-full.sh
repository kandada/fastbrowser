#!/usr/bin/env bash
# 真 Chromium 重型集成测试（默认不跑，避免慢/在负载下偶发超时）。
#   bash scripts/cdp-full.sh
# 需要本机有 Chrome/Chromium（或 CHROME_PATH）。
set -euo pipefail
cd "$(dirname "$0")/.."
echo "==> heavy real-Chromium tests (feature heavy-tests)"
cargo test --features engine-cdp,heavy-tests

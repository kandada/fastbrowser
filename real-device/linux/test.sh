#!/bin/bash
# Linux 真实浏览器测试（headless / headed）。用法： real-device/linux/test.sh [headless|headed]
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FB_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"   # fastbrowser 内核仓库
source "$SCRIPT_DIR/../env.sh"
MODE="${1:-headless}"
cd "$FB_ROOT"

# 无 X 时用 Xvfb（可选）
if [ "$MODE" = "headed" ] && [ -z "${DISPLAY:-}" ]; then
  command -v xvfb-run >/dev/null && exec xvfb-run -a "$0" headed
fi

cargo test --features "engine-cdp,async-core" --test chromium_integration --test chromium_headed_integration --test async_core_e2e
bash scripts/cleanup-orphan-browsers.sh
echo "done ($MODE)"

#!/usr/bin/env bash
# 收割孤儿 fastbrowser 浏览器进程 + 临时目录（部署与测试兜底）。
#
# 背景：
# - `engine: "bundled"` 启动的 Chrome for Testing 随引擎 drop 自动清理；
# - 测试用 Chrome 由 tests/common 的共享浏览器管理（进程级单例），
#   测试进程退出后 Chrome 成为孤儿，由下次测试启动时的 reap 收割；
# 但若宿主/Agent/测试进程被 SIGKILL / OOM 强杀，Chrome 会残留为孤儿进程
# 持续占用 CPU/内存。本脚本按 fastbrowser 的独有启动标记收割进程，并清理
# 临时 user-data 目录与租约文件，不影响用户自己打开的普通 Chrome。
#
# 用法：
#   scripts/cleanup-orphan-browsers.sh            # 清理本机全部孤儿 fastbrowser 浏览器
#   scripts/cleanup-orphan-browsers.sh --dry-run  # 只列出匹配项，不杀
#
# 建议：放入 crontab / launchd 定期执行，或集成到应用退出流程 / CI 后置步骤。
set -euo pipefail

DRY=0
if [[ "${1:-}" == "--dry-run" ]]; then
    DRY=1
fi

# 匹配 fastbrowser 独有启动标记（pgrep -f 使用 ERE，交替用单竖线）：
#   fastbrowser-cft-*            → bundled 引擎临时 user-data-dir
#   fastbrowser-test-chrome-*    → 集成测试临时 user-data-dir
# 普通 Chrome 不会使用这些目录。
# 注意：匹配串不能以 "-" 开头，否则会被 pgrep 解析为选项。
MATCH='fastbrowser-cft-|fastbrowser-test-chrome-'

TMP="${TMPDIR:-/tmp}"

if [[ "$DRY" == "1" ]]; then
    echo "[dry-run] 将收割以下孤儿 fastbrowser Chromium 进程："
    pgrep -fl "$MATCH" || echo "  （无匹配进程）"
    echo "[dry-run] 将清理以下临时目录/租约文件："
    find "$TMP" -maxdepth 1 \
        \( -name 'fastbrowser-test-chrome-*' -o -name 'fastbrowser-cft-*' -o -name 'fastbrowser-test-chrome.lease' \) \
        -print 2>/dev/null || true
    exit 0
fi

# 1. 清理孤儿进程
pids=$(pgrep -f "$MATCH" || true)
if [[ -n "$pids" ]]; then
    echo "cleanup: 收割孤儿进程: $pids"
    pkill -9 -f "$MATCH" || true
    sleep 1
else
    echo "cleanup: 无孤儿 fastbrowser Chromium 进程"
fi

# 2. 清理临时 user-data 目录
dirs=$(find "$TMP" -maxdepth 1 \( -name 'fastbrowser-test-chrome-*' -o -name 'fastbrowser-cft-*' \) -print 2>/dev/null || true)
if [[ -n "$dirs" ]]; then
    echo "cleanup: 清理临时 user-data 目录："
    echo "$dirs" | sed 's/^/  /'
    find "$TMP" -maxdepth 1 \( -name 'fastbrowser-test-chrome-*' -o -name 'fastbrowser-cft-*' \) \
        -exec rm -rf {} + 2>/dev/null || true
fi

# 3. 清理租约文件（残留的测试进程租约）
rm -f "$TMP/fastbrowser-test-chrome.lease"

# 4. 复查
left=$(pgrep -f "$MATCH" || true)
if [[ -n "$left" ]]; then
    echo "cleanup: 仍残留进程: $left（可能需要提升权限）" >&2
    exit 1
fi
echo "cleanup: 完成"

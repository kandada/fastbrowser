#!/bin/bash
# ═══════════════════════════════════════════════════════════
# 校验 fastbrowser.h 与 C ABI 符号是否一致：
#   1. 编译内核 cdylib
#   2. 对照 fastbrowser.h 声明的函数名，检查导出符号
# ═══════════════════════════════════════════════════════════

set -euo pipefail
cd "$(dirname "$0")/.."
FB_LOCAL="$(cd "$(dirname "$0")/.." && pwd)"   # fastbrowser 仓库根（target 存放于此）

echo "=== cargo build --release ==="
cargo build --release

LIB=$(find "$FB_LOCAL/target/release" -maxdepth 1 \( -name 'libfastbrowser.dylib' -o -name 'libfastbrowser.so' -o -name 'fastbrowser.dll' \) 2>/dev/null | head -1)
if [ -z "$LIB" ]; then
  echo "error: no shared library found in target/release" >&2
  exit 1
fi
echo "library: $LIB"

echo "=== 检查头文件声明的符号 ==="
perl -ne 'while(/(fastbrowser_[a-z0-9_]+)\s*\(/g){print "$1\n"}' fastbrowser_c/include/fastbrowser.h | sort -u | while read -r sym; do
  if nm -gU "$LIB" 2>/dev/null | grep -q "_${sym}$"; then
    echo "  ok   $sym"
  else
    echo "  MISS $sym"
  fi
done

echo "=== 语法检查头文件 ==="
cc -fsyntax-only -std=c11 fastbrowser_c/include/fastbrowser.h && echo "  ok fastbrowser.h"
echo "Done."

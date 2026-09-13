# Linux 真机测试（P2，无头 CI 优先）

## 环境（大资源在外部盘）

```bash
source real-device/env.sh
# Chrome for Testing (linux64) → $FB_LOCAL/vendor/chromium-linux/
./scripts/fetch-chromium.sh   # 已在脚本内按平台选择 linux64
```

## 运行

```bash
# 无头真实浏览器测试（CI 可用，无 GUI）
real-device/linux/test.sh headless

# 有 GUI 会话时跑有头
real-device/linux/test.sh headed

# 参考 runner（chromium）
cd real-device/shared/runner && cargo run --features engine-cdp -- --cases ../cases --engine chromium
```

## 重点
- 无显示器（纯 headless）下截图/快照/下载全部可用（不依赖 X）。
- `Xvfb :99` 可替代真实显示器跑 headed 类用例。
- 进程清理沿用 `scripts/cleanup-orphan-browsers.sh`（Linux 直接可用）。

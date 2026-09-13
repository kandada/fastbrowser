# Windows 真机测试（P1，打包 Chromium + CDP）

复用内核已验证的 `bundled` 引擎（打包 Chrome for Testing + CDP），在 Windows 上跑
`chromium_integration` / `chromium_headed` / `async_core_e2e` 三套真实浏览器测试。

## 环境（大资源在外部盘）

| 资源 | 位置 | 说明 |
|---|---|---|
| Rust toolchain | 外部盘 `toolchain/`（或用 rustup 默认） | `rustup default stable-x86_64-pc-windows-msvc` |
| Chrome for Testing (win64) | `$FB_LOCAL/vendor/chromium-win/` | `env.ps1` 设 `CHROME_PATH` |
| Rust target | `$FB_LOCAL/target/` | `.cargo/config.toml` 已重定向 |
| 报告 | `$FB_LOCAL/real-device-reports/windows/` | runner 输出 |

## 运行

```powershell
# 1) 下载 Chrome for Testing（~150MB）到外部盘
powershell -File real-device/windows/fetch-chromium.ps1

# 2) 跑真实浏览器集成测试（无头 + 有头）
powershell -File real-device/windows/test.ps1

# 3) 可选：参考 runner 跑 JSON 用例（chromium）
cd real-device\shared\runner
cargo run --features engine-cdp -- --cases ..\cases --engine chromium
```

## 重点验证

- `bundled` 启动（无头拉起 + 连接）；有头窗口渲染（headed 套件）。
- 路径分隔符 / 长路径 / 中文路径下载文件名编码。
- Windows 进程回收（`cleanup-orphan-browsers.sh` 的 Windows 对应：`taskkill /F /FI "WINDOWTITLE eq *fastbrowser-test-chrome*"` 或按命令行匹配）。

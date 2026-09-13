# 桌面 CEF 嵌入（engine-cef）验证

`engine-cef` 是桌面端"内嵌浏览器"（windowless CEF + CDP 驱动）路径；本目录负责拉取 CEF 预编译包
（**大资源放外部盘**）并给出验证步骤。CEF 在真实桌面宿主中尚未验证（见 real-device-test-plan §7）。

## 拉取 CEF（→ 外部盘 vendor/cef）

```bash
source real-device/env.sh
bash real-device/cef/fetch-cef.sh            # 默认 stable / 本机架构
# 产物：$FB_LOCAL/vendor/cef/（约 150MB，BSD 许可，随包需保留 license + about:license）
```

## 构建 engine-cef

```bash
cargo build --release --features engine-cef --manifest-path fastbrowser/Cargo.toml
```

## 验证步骤（有 GUI 的桌面环境）

1. 冒烟：`CefEngine::new(config)` → `create_tab` → `navigate` → `snapshot`（视图托管 windowed/OSR）。
2. OSR 帧流：`start_frame_stream` 推帧到宿主预览窗。
3. 复用 CDP 全部语义：点击/快照/下载/拦截/历史（协议层与 bundled 一致）。
4. 随包义务：保留 CEF license 与 `about:license` 入口。

## 说明
- `engine-cef` = `engine-cdp` + CEF 宿主（`engines/cef.rs`）；编译需 `vendor/cef` + cmake/ninja。
- 若只做"打包 Chromium 集成"验证，用 `bundled` 引擎即可（更简单、已随宿主应用验证过）。

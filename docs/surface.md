# Surface（电脑操作）层

> 状态：v0.1 已实现（异步优先）。设计思想见仓库外 `FASTBROWSER_ACCESSIBILITY.md`。
> 目标：把 fastbrowser 从「浏览器自动化内核」扩展为「可嵌入的电脑操作内核」，
> 浏览器是第一个、也是最成熟的 surface。

## 一句话

统一 `UiNode` 模型 + 异步 `SurfaceProvider` + provider 注册表 + 分层工具 + feature 门控；
浏览器（web 表面）、macOS 原生无障碍（desktop 表面）与脚本化 mock 都产出同一形状快照，
Agent 用同一套 `surface_*` 工具跨表面操作。

## Feature 与构建

| feature | 内容 | 依赖 |
|---|---|---|
| `surface` | 核心模型 + 异步 provider/注册表 + 浏览器与 mock provider + `surface_*` 工具 | tokio, async-trait |
| `surface-macos` | macOS 原生无障碍（AXUIElement）+ 坐标输入（CGEvent）+ 原生截图（screencapture） | `surface` + accessibility/accessibility-sys/core-foundation/core-graphics |
| `surface-linux` | Linux 原生无障碍（AT-SPI via zbus）；仅 Linux 运行，可跨平台编译校验 | `surface` + atspi |
| `surface-windows` | Windows 原生无障碍（UI Automation）；仅 Windows 运行 | `surface` + uiautomation |
| `surface-android` | Android 原生无障碍；由宿主（Kotlin `AccessibilityService`）经 JNI/C ABI `FbSurfaceOps` 注入 ops | `surface` |

```bash
# 仅浏览器表面（跨平台）
cargo build --features surface

# 浏览器 + macOS 原生表面
cargo build --features surface-macos

# Linux 原生后端（类型检查）
cargo check --features surface-linux

# Windows 原生后端（需 Windows target + Windows SDK）
cargo check --target x86_64-pc-windows-msvc --features surface-windows

# 测试
cargo test --features surface
cargo test --features surface-macos
```

默认 feature 不含 surface，**只做浏览器的用户零增量**（无 tokio、无平台 AX 依赖）。

## 架构

```
engine/surface.rs      # UiNode / SurfaceSnapshot / SurfaceRef / SurfaceAction / 裁剪（零平台依赖，永远编译）
engine/provider.rs     # async SurfaceProvider trait + ProviderRegistry + SurfaceRuntime（同步外壳）
surfaces/browser.rs    # BrowserEngine -> SurfaceProvider（web）
surfaces/mock.rs       # 脚本化表面（测试/无平台）
surfaces/macos.rs      # macOS 原生（desktop，feature surface-macos）
tools/surface.rs       # ax_list / ax_snapshot / ax_act / ax_click / ax_type / ax_scroll / ax_events
```

- **异步核心 + 同步外壳**：provider 全异步；`SurfaceRuntime` 持有一个 tokio 运行时，
  向同步工具/C ABI 暴露阻塞 API（`*_blocking`），每次调用套 `tokio::time::timeout`。
  绝不在异步上下文里调用 `*_blocking`。
- **路由**：ref 形如 `<namespace>:<surface>:<id>`（`web:3:a` / `desktop:1234:0.2.1`），
  注册表按命名空间分发。
- **能力门控**：`Runtime::effective_caps()` 在引擎能力上叠加 `supports_surface`；
  surface 工具声明 `Capability::Surface`，未装配时自动隐藏并明确报错。
- **C ABI / SDK 自动暴露**：新工具经既有 `fastbrowser_tool_call` / `Fastbrowser::tool_call` 即可用，无需新增 FFI。

## 统一模型

`UiNode`：`ref` / `role` / `name` / `value` / `states` / `geometry` / `actions` / `children` / `source` / `backend`。
`SurfaceSnapshot`：`surface` + `root` + `generation` + `meta`（含 `truncated`/`reason`）。

## 方言与别名（与浏览器工具一致）

`ax_*` 工具的元素目标支持与浏览器专有工具**完全相同的方言**：

| 形式 | 示例 | 说明 |
|---|---|---|
| 完整 ref | `web:3:a` / `desktop:1234:0.2.1` | 直接路由到对应 provider |
| 裸 ref | `a` / `#go` / `text=Go` / `role=button` / `//button` | 自动绑定到活动/首个表面 |
| Playwright `eN` | `e1` / `e5` | `e`+数字（1-based）→ 第 N 个可交互元素（映射到快照字母） |
| `{kind,value}` 对象 | `{"kind":"css","value":"#go"}` / `{"kind":"snapshot","value":"e3"}` | 与浏览器工具同义 |
| `selector` 参数 | `{"selector":"role=button"}` | Playwright 风格选择器方言 |
| `id` 参数 | `{"id":"a"}` / `{"id":"submit"}` / `{"id":3}` | 单字母/`eN`→快照；数字→快照下标；其余→CSS/`#id` |
| `element` 参数 | `{"element":"Submit"}` | 可读描述→文本匹配 |
| 坐标 | `{"x":120,"y":80}` | 坐标点击（`ax_click` / `ax_act`），优先于元素目标 |

选择器方言解析统一下沉到 `engine::snapshot::parse_selector_dialect`（浏览器工具与
surface 工具共用一份实现）：`css=` / `text=` / `role=` / `xpath=` / `id=` /
`data-testid=`（`testid=` 同义）/ `nth=N` / `//...` / `:has-text("...")`，其余按 CSS。
快照引用识别（单字母小写 `a..z` 与 `eN`）在 `engine::snapshot::parse_snapshot_ref`。

### Playwright 选择器引擎

含 Playwright 专有语法时（`>>` / `:visible` / `:has-text()` / `:text()` /
`:nth-match()` / `role=...[name="..."]`），解析为 `RefKind::Selector`，由注入 JS
引擎（`engine::inject::fb_query_js`）在真实 DOM 上解析。支持子集：

- **链式** `A >> B`（B 在 A 的后代中查找；`nth=N` 段取当前结果第 N 个，0-based）
- **段引擎前缀**：`css=` / `text=` / `xpath=` / `id=` / `data-testid=`(`testid=`) / `role=`
- **`role=button[name="X"]`**：按可访问名过滤
- **CSS 伪类**：`:visible`、`:has-text("...")`、`:text("...")`（`<has>`/`:not()` 走原生 CSS）
- **文本**：`text=foo`（子串、忽略大小写）、`text="exact"`、`text=/regex/`
- **快照引用**：单字母 / `eN`（Playwright MCP 风格）

mock 引擎无 DOM 层级，链式近似取最后一段，其余（引擎前缀/伪类/nth）语义一致，用于
确定性测试。真 Chromium 覆盖见 `tests/selector_engine_cdp.rs`。

> 说明：完整 Playwright 的 `internal:` 引擎、`>>` 内嵌 `nth=` 的复杂组合、跨 shadow
> 根的 Playwright 专属语义未实现（走原生 CSS 能覆盖的部分仍可用）。

规范工具名以 `ax_` 前缀暴露 Accessibility API；旧名与各生态名作为别名解析
（feature `surface` 生效，见 `tools::aliases::SURFACE_TOOL_ALIASES`）：
`list_windows`/`list_apps`/`surface_list`/`list_surfaces` → `ax_list`；
`ui_snapshot`/`desktop_snapshot`/`surface_snapshot`/`computer_snapshot` → `ax_snapshot`；
`desktop_click`/`ui_click`/`surface_click` → `ax_click`；
`desktop_type`/`ui_type`/`surface_type` → `ax_type`；
`desktop_scroll`/`ui_scroll`/`surface_scroll` → `ax_scroll`；
`surface_events` → `ax_events`；
`desktop_act`/`ui_act`/**`computer`**/`surface_act` → `ax_act`。
参数别名：`surface`/`window` → `target`，`node` → `ref`，**`index` → `id`**（browser-use）。
`ax_act` 的 `action` 还接受 Anthropic computer-use 风格名：`left_click`/`right_click`/
`middle_click`/`key`。

## Token 预算（裁剪）

`SnapshotOptions`（默认 `interesting_only=true, max_depth=12, max_nodes=300, max_text_len=200`）：

1. 字段截断；
2. 后序裁剪：丢弃无 name/value/action 的纯容器（但保留有子树的结构节点）；
3. 深度上限 → `reason=depth`；
4. 节点上限 → `reason=nodes`。

截断永远显式可见（`meta.truncated` + `meta.reason` + 真实总数）。

## 配置

```jsonc
{
  "surface": {
    "enabled": true,
    "provider": "auto",       // auto | browser | macos | mock | none
    "max_nodes": 0,           // 0 = 内核默认
    "max_depth": 0,
    "interesting_only": true,
    "timeout_ms": 0           // 0 = 默认 20s
  }
}
```

## 工具

| 工具 | 说明 |
|---|---|
| `ax_list` | 枚举所有 provider 的表面（浏览器标签 + 原生应用） |
| `ax_snapshot` | 取统一 `UiNode` 快照（`target` 可选；默认首个表面） |
| `ax_act` | 对 `ref` 施加动作：click/double_click/right_click/focus/set_value/type/check/select/scroll/press_key/increment/decrement/show_menu/raise |
| `ax_click` / `ax_type` / `ax_scroll` | 常用动作快捷方式 |
| `ax_events` | drain 原生无障碍事件（焦点/窗口/值/标题/选择/结构） |

## macOS 原生后端

- 权限：`AXIsProcessTrusted`；未授予时返回明确错误（不静默失败）。
- 快照：聚焦应用（或 `desktop:<pid>`）的 AX 树，应用根用 `AXWindows`、其余用 `AXChildren`
  作为确定性子节点顺序；每个元素设 AX messaging timeout，避免无响应应用卡死。
- ref：`desktop:<pid>:<path>`，`path` 为子节点下标（`0.2.1`）；动作时按 path 重新走树解析。
- 动作：`AXPress`/`AXIncrement`/`AXDecrement`/`AXShowMenu`/`AXRaise`/`AXValue` 写入/`AXFocused`；
  双击/右键/滚动/按键走 `CGEvent`。
- **截图**：`screencapture -x -t png` → 内核 PNG 解码；需「屏幕录制」权限，未授权明确报错。

## Linux 原生后端（AT-SPI）

- 通过 `org.a11y.atspi.*`（zbus）访问：`Accessible`（角色/名称/子节点/状态）、
  `Component`（屏幕几何）、`Action`（点击）、`Value`/`EditableText`（设值/文本）。
- ref：`desktop:<appIndex>:<child.path>`。仅在 Linux 注册；其他平台可编译校验。
- 需桌面会话启用无障碍（`gsettings set org.gnome.desktop.interface toolkit-accessibility true`）。

## Windows 原生后端（UI Automation）

- 通过 `uiautomation` 访问：ControlType/名称/几何/启用/焦点；动作走 Invoke/Value/Toggle
  模式或元素级 click/focus。COM 在每次阻塞调用内初始化。
- ref：`desktop:<topIndex>:<child.path>`。仅在 Windows 注册。
- 交叉检查需 Windows target + Windows SDK（`ring` 的 C 构建依赖 SDK）；本仓已用隔离探针
  验证 `uiautomation` API 用法。

## 浏览器表面：CDP AX 统一 + 几何关联

- `prefer_ax_tree=true`（默认）时，`ax_snapshot` 走引擎原生 AX 树
  （CDP `Accessibility.getFullAXTree`），保留层级；每个节点附 `backendDOMNodeId`，
  经 `DOM.getBoxModel` 关联几何。引用为 `web:<tab>:ax:<backendId>`。
- `ax_click` 等对 AX 引用按**几何中心坐标**注入（`Input.dispatchMouseEvent`），
  文本输入为「点击聚焦 + `Input.insertText`」。
- 关闭 `prefer_ax_tree` 时回退到注入 JS 的交互元素快照（旧行为）。

## 事件流（原生通知）

`ax_events` 工具 drain 自上次调用以来的原生事件（`SurfaceEvent`：focus/window/value/title/selection/structure）。
- **macOS**：首次调用为当前聚焦应用启动 `AXObserver`，在独立 CFRunLoop 线程收集通知并回调入队（队列上限 1024，Drop 时停 runloop）。无「辅助功能」权限时返回空列表。
- 其它 provider 默认无事件源（返回空）；mock 提供 `push_event` 供测试。
- 已知限制：观察对象是**启动时的聚焦应用**，跨应用切换不会自动重新注册（后续可在 `AXApplicationActivated` 时重注册）。

## 跨平台编译验证

`surface-*` 后端各自只能在对应平台编译，交叉验证方式：

```bash
# Linux（cargo-zigbuild：zig 作 C 编译器/链接器，免 Docker）
cargo zigbuild --target aarch64-unknown-linux-gnu --features surface-linux --lib --tests
cargo zigbuild --target x86_64-unknown-linux-gnu  --features surface-linux --lib

# Windows（zig 直接编 windows-gnu；见 zig-tools 包装器过滤 --target=）
CC_x86_64_pc_windows_gnu=<zig-cc-win> AR_x86_64_pc_windows_gnu=<zig-ar> \
  cargo check --target x86_64-pc-windows-gnu --features surface-windows --tests

# 或 CI 用 cross-rs（Docker+QEMU）
cross check --target x86_64-pc-windows-gnu --features surface-windows
```

## 尚未实现（明确的下一步）

- **事件流的跨应用重注册**：当前只观察启动时的聚焦应用。
- **已知依赖告警**：`surface-macos` 经 `accessibility → cocoa → objc 0.2 → block 0.1.6`
  带出一个 future-incompat 告警（非错误）；后续可改用 `objc2` + 原始 AX 消除。

## 测试

- 单元：`engine::surface`（裁剪/ref/序列化/动作 round-trip/边界）、`engine::provider`
  （路由/能力/同步外壳/超时/部分失败/输入路由）、`engine::snapshot`（方言解析）、
  `surfaces::{browser,mock,macos,linux,windows}`（各后端 + 方言 + 表面级动作 + 树组装）。
- 集成：`tests/surface.rs`（工具发现、能力门控、路由、裁剪、动作回环、浏览器/原生/脚本表面、
  方言、别名、`{kind,value}` ref、status、表面级动作）。
- 真浏览器：`tests/surface_cdp.rs`（feature `engine-cdp,surface`）：AX 快照产出 `ax:` 引用 +
  几何，并以 AX 引用坐标点击触发真实页面行为；`prefer_ax_tree=false` 回退。
- 全量回归：`cargo test`（默认）、`cargo test --features surface`、
  `cargo test --features surface-macos`、`cargo test --features engine-cdp`（真 Chromium）。
- 质量门：`cargo fmt --all -- --check`、`scripts/check-english.py`、
  `cargo clippy --all-targets`（含各 feature）无新增告警。

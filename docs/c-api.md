# C ABI（fastbrowser.h）

## 总览

一套头文件、三平台同 ABI。由 `libfastbrowser.{a,dylib,so}` 导出（Rust）。

- **内存**：所有返回 `char*` 的函数分配 Rust 堆字符串，宿主用
  `fastbrowser_free_string` 释放。
- **错误**：`{"error":{"kind":"...","message":"..."}}`。
- **线程**：全部函数线程安全；内核内部互斥序列化。

## 生命周期

```c
char *fastbrowser_version(void);
char *fastbrowser_init(const char *config_json);   // Config 的 JSON
char *fastbrowser_shutdown(void);
void  fastbrowser_free_string(char *ptr);
```

## 导航 / 工具

```c
char *fastbrowser_open(const char *url);
char *fastbrowser_navigate(const char *url);
char *fastbrowser_tool_list(void);
char *fastbrowser_tool_call(const char *name, const char *params_json);
```

## 页面 / 视图

```c
char *fastbrowser_snapshot(void);
char *fastbrowser_screenshot(void);          // {width,height,format:"rgba",base64}
char *fastbrowser_get_view(void);
char *fastbrowser_set_viewport(uint32_t width, uint32_t height);
```

## 信息 / 权限 / 审计

```c
char *fastbrowser_get_info(void);
char *fastbrowser_status(void);
void  fastbrowser_set_permission(const char *resource, unsigned char allowed);
char *fastbrowser_audit(void);       // 动作审计日志（JSON 数组，新→旧）
char *fastbrowser_clear_audit(void); // 清空审计
```

## 回调

```c
typedef void (*fastbrowser_event_callback)(uint32_t tab, const char *event_json);
int fastbrowser_register_event_callback(fastbrowser_event_callback cb);

typedef void (*fastbrowser_viewframe_callback)(uint32_t tab, const char *frame_json);
int fastbrowser_register_viewframe_callback(fastbrowser_viewframe_callback cb);
```

## 移动端 WebViewOps

宿主填充 `FbWebViewOps` 函数表，经 `fastbrowser_register_webview_ops` 注入，
内核 `engine=webview` 即通过该表驱动系统 WebView。

## 平台接入

- **Android**：`fastbrowser_c/build.sh` → staticlib；NDK CMake → `libfastbrowser_jni.so`；
  Kotlin `com.fastbrowser.Sdk`（见 `bindings/android`）。
- **iOS/macOS**：链接 `libfastbrowser.a`，直接调用。
- **桌面**：cdylib，`dlopen` 或链接。

## 校验

```bash
./scripts/gen-c-headers.sh   # 对照头文件检查导出符号 + 语法
```

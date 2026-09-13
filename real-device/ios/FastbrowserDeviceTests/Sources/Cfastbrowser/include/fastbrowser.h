/*
 * Copyright (c) 2025 xiefujin <490021684@qq.com>
 * Licensed under Apache-2.0, see LICENSE file for full license terms.
 *
 * fastbrowser.h — Pure C ABI exported by libfastbrowser (Rust staticlib/cdylib).
 *
 * One header. All platforms. Same C ABI everywhere.
 *
 * ── Platform integration paths ──
 *
 *   ANDROID (staticlib + NDK CMake → libfastbrowser_jni.so)
 *     Rust → libfastbrowser.a (extern "C" capi symbols)
 *           ↓ NDK clang + CMake
 *     jni_glue.c → Java_com_fastbrowser_Sdk_native* → libfastbrowser_jni.so
 *           ↓ System.loadLibrary("fastbrowser_jni")
 *     Kotlin  (com.fastbrowser.Sdk)
 *
 *   iOS/macOS (static linking → app binary)
 *     Rust → libfastbrowser.a
 *           ↓ Xcode "Link Binary with Libraries"
 *     Swift/ObjC → direct C function calls
 *
 *   DESKTOP (macOS / Windows / Linux)
 *     Rust → libfastbrowser.{dylib,dll,so} (cdylib) or staticlib
 *     C host → direct C function calls (or dlopen at runtime)
 *
 * ── Memory ownership ──
 *   Functions returning `char *` allocate a NUL-terminated UTF-8 string on the
 *   Rust heap. The caller MUST release it with fastbrowser_free_string().
 *   All results are JSON. Errors look like:
 *       {"error":{"kind":"...","message":"..."}}
 *
 * ── Thread safety ──
 *   All functions are safe to call from any thread. The kernel uses an internal
 *   Mutex to serialize access. Event/frame callbacks are invoked on the caller's
 *   thread that triggered them.
 */

#ifndef FASTBROWSER_H
#define FASTBROWSER_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ── Lifecycle ─────────────────────────────────────────────── */

/*
 * Returns the kernel version string (malloc'd, free with fastbrowser_free_string).
 */
char *fastbrowser_version(void);

/*
 * Initializes the kernel. config_json is the JSON form of the Config struct,
 * e.g. {"engine":"mock"}. Fields may be omitted (defaults apply).
 * Returns JSON: {"ok":true} or {"error":{...}}.
 */
char *fastbrowser_init(const char *config_json);

/*
 * Shuts the kernel down and releases the engine.
 * Returns JSON: {"ok":true}.
 */
char *fastbrowser_shutdown(void);

/*
 * Frees a string previously returned by any fastbrowser_* function.
 */
void fastbrowser_free_string(char *ptr);

/* ── Navigation ────────────────────────────────────────────── */

/* Opens a URL in a new tab and activates it. Returns JSON with tab/url/title. */
char *fastbrowser_open(const char *url);

/* Navigates the active tab (auto-creating one if needed). */
char *fastbrowser_navigate(const char *url);

/* ── Agent tools ───────────────────────────────────────────── */

/* Returns the full tool manifest (JSON array of specs) for the LLM. */
char *fastbrowser_tool_list(void);

/*
 * Calls a tool by name with a JSON params object (string).
 * e.g. fastbrowser_tool_call("click", "{\"id\":\"a\"}").
 * Returns the tool's JSON result.
 */
char *fastbrowser_tool_call(const char *name, const char *params_json);

/* ── Page / view ───────────────────────────────────────────── */

/* Returns the interactive-element snapshot (JSON PageSnapshot). */
char *fastbrowser_snapshot(void);

/* Returns the screenshot as JSON: {width,height,format:"rgba",base64}. */
char *fastbrowser_screenshot(void);

/* Returns the host view handle for the active tab: {"view":{...}} or {"view":null}. */
char *fastbrowser_get_view(void);

/* Sets the viewport. Returns JSON {"ok":true}. */
char *fastbrowser_set_viewport(uint32_t width, uint32_t height);

/*
 * Starts continuous frame push to the view-frame callback.
 * tab is the target tab id; opts_json is JSON {fps, max_width, max_height}
 * (all optional). Frames arrive via fastbrowser_register_viewframe_callback.
 * Returns JSON {"started":true,...} or an error (engine unsupported).
 */
char *fastbrowser_start_frame_stream(uint32_t tab, const char *opts_json);

/* Stops frame push for a tab. Returns JSON {"stopped":true}. */
char *fastbrowser_stop_frame_stream(uint32_t tab);

/* ── Info / status ─────────────────────────────────────────── */

/* Returns kernel info: version, engine, initialized, tools, etc. */
char *fastbrowser_get_info(void);

/* Returns a status summary: engine, tabs, active_tab, profiles, tools. */
char *fastbrowser_status(void);

/* Returns the action audit log as a JSON array (newest first). */
char *fastbrowser_audit(void);

/* Clears the action audit log. */
char *fastbrowser_clear_audit(void);

/* ── Permissions (mirrors fastshell) ───────────────────────── */

/*
 * Grants (allowed != 0) or denies a permission for a resource key.
 * Keys: "network:<host>", "camera", "location", "microphone", ...
 * The host app should map these to platform permissions.
 */
void fastbrowser_set_permission(const char *resource, unsigned char allowed);

/* ── Streaming callbacks ───────────────────────────────────── */

/*
 * Registers a page-event callback (or clears it with NULL).
 * cb(tab, event_json) is invoked for navigation/console/network events.
 * Register BEFORE fastbrowser_init to attach to early tabs.
 */
typedef void (*fastbrowser_event_callback)(uint32_t tab, const char *event_json);
int fastbrowser_register_event_callback(fastbrowser_event_callback cb);

/*
 * Registers an off-screen frame callback (or clears it with NULL).
 * cb(tab, frame_json) receives {width,height,seq,base64}.
 */
typedef void (*fastbrowser_viewframe_callback)(uint32_t tab, const char *frame_json);
int fastbrowser_register_viewframe_callback(fastbrowser_viewframe_callback cb);

/* ── Mobile WebView ops (host implements real WebView) ─────── */

/*
 * Function table the host fills in to drive a system WebView
 * (Android WebView / iOS WKWebView / Harmony ArkWeb). Register BEFORE init
 * when using engine "webview".
 *
 * Memory contract:
 *   - evaluate returns a malloc'd JSON string; host releases via free_string.
 *   - screenshot writes width/height and returns a malloc'd RGBA buffer;
 *     host releases via screenshot_free.
 *   - Return negative handles / non-zero codes on failure.
 */
typedef struct FbWebViewOps {
    /* Create a native webview. Returns handle (>=0) or -1. */
    int64_t (*create)(const char *url, uint32_t width, uint32_t height);
    /* Destroy a webview. */
    int (*destroy)(int64_t handle);
    /* Evaluate JS; returns malloc'd JSON string or NULL on failure. */
    char *(*evaluate)(int64_t handle, const char *script);
    /* Navigate the webview. */
    int (*navigate)(int64_t handle, const char *url);
    /* Capture screenshot; returns malloc'd RGBA buffer (free via screenshot_free). */
    uint8_t *(*screenshot)(int64_t handle, uint32_t *out_w, uint32_t *out_h);
    void (*screenshot_free)(uint8_t *ptr);
    /* Set viewport. */
    int (*set_viewport)(int64_t handle, uint32_t width, uint32_t height);
    /* Return a native view handle for embedding (>=0) or -1. */
    int64_t (*native_view)(int64_t handle);
    /* Dispatch an input event given as JSON. */
    int (*dispatch_event)(int64_t handle, const char *event_json);
    /* History navigation. */
    int (*go_back)(int64_t handle);
    int (*go_forward)(int64_t handle);
    /* Free a string returned by evaluate. */
    void (*free_string)(char *ptr);
} FbWebViewOps;

/* Registers the host WebView ops table. Returns 0 on success. */
int fastbrowser_register_webview_ops(const FbWebViewOps *ops);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* FASTBROWSER_H */

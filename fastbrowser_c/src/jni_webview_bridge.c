/*
 * Copyright (c) 2025 xiefujin <490021684@qq.com>
 * Licensed under Apache-2.0, see LICENSE file for full license terms.
 *
 * jni_webview_bridge.c — Android WebViewOps 桥（真机测试用，独立于 jni_glue.c）。
 *
 * 让 Kotlin 实现真实 WebView（Android WebView = Chromium），通过 FbWebViewOps 函数表
 * 注入内核：
 *
 *   Kotlin: com.fastbrowser.WebViewBridge 静态方法（持有 WebView 实例）
 *      ↑ JNI
 *   本文件: FbWebViewOps 函数表（各回调 → Java 静态方法）
 *      ↑
 *   Rust: fastbrowser_register_webview_ops(ops)
 *
 * 由 real-device/android/build.sh 与 fastbrowser_c/src/jni_glue.c 一起编译进
 * libfastbrowser_jni.so。Sdk.nativeRegisterWebViewOps() 在 init 前调用。
 */

#include <jni.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "fastbrowser.h"

/* ── Java 类引用 ─────────────────────────────────────────────── */

static JavaVM *g_vm = NULL;
static jclass g_wb = NULL;
static jmethodID m_create = NULL, m_destroy = NULL, m_evaluate = NULL, m_navigate = NULL;
static jmethodID m_screenshot = NULL, m_set_viewport = NULL, m_native_view = NULL;
static jmethodID m_dispatch = NULL, m_go_back = NULL, m_go_forward = NULL;

static jstring js_new(JNIEnv *env, const char *s) {
    return (*env)->NewStringUTF(env, s ? s : "");
}

static JNIEnv *env_get(void) {
    JNIEnv *env = NULL;
    if (g_vm == NULL) return NULL;
    if ((*g_vm)->GetEnv(g_vm, (void **)&env, JNI_VERSION_1_6) != JNI_OK) {
        if ((*g_vm)->AttachCurrentThread(g_vm, (JNIEnv **)&env, NULL) != JNI_OK) return NULL;
    }
    return env;
}

/* ── FbWebViewOps 回调 ────────────────────────────────────────── */

static int64_t op_create(const char *url, uint32_t w, uint32_t h) {
    JNIEnv *env = env_get();
    if (env == NULL) return -1;
    jstring ju = js_new(env, url);
    jlong hnd = (*env)->CallStaticLongMethod(env, g_wb, m_create, ju, (jint)w, (jint)h);
    (*env)->DeleteLocalRef(env, ju);
    return (int64_t)hnd;
}

static int op_destroy(int64_t handle) {
    JNIEnv *env = env_get();
    if (env == NULL) return -1;
    return (*env)->CallStaticIntMethod(env, g_wb, m_destroy, (jlong)handle);
}

static char *op_evaluate(int64_t handle, const char *script) {
    JNIEnv *env = env_get();
    if (env == NULL) return NULL;
    jstring js_script = js_new(env, script);
    jstring out = (jstring)(*env)->CallStaticObjectMethod(env, g_wb, m_evaluate, (jlong)handle, js_script);
    (*env)->DeleteLocalRef(env, js_script);
    if (out == NULL) return NULL;
    const char *c = (*env)->GetStringUTFChars(env, out, NULL);
    char *dup = c ? strdup(c) : NULL;
    (*env)->ReleaseStringUTFChars(env, out, c);
    (*env)->DeleteLocalRef(env, out);
    return dup;
}

static int op_navigate(int64_t handle, const char *url) {
    JNIEnv *env = env_get();
    if (env == NULL) return -1;
    jstring ju = js_new(env, url);
    int r = (*env)->CallStaticIntMethod(env, g_wb, m_navigate, (jlong)handle, ju);
    (*env)->DeleteLocalRef(env, ju);
    return r;
}

/* 最小 base64 解码（标准表）；返回 malloc 缓冲，失败返回 NULL。 */
static uint8_t *b64_decode(const char *s, size_t len, size_t *out_len) {
    static const int8_t T[256] = {0};
    static int8_t init = 0;
    if (!init) {
        const char *abc = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        for (int i = 0; i < 64; i++) T[(uint8_t)abc[i]] = (int8_t)i;
        init = 1;
    }
    size_t cap = (len / 4) * 3 + 3;
    uint8_t *out = (uint8_t *)malloc(cap + 1);
    if (!out) return NULL;
    size_t o = 0, v = 0, n = 0;
    for (size_t i = 0; i < len; i++) {
        int8_t c = T[(uint8_t)s[i]];
        if (s[i] == '=' || c < 0) continue;
        v = (v << 6) | (uint8_t)c;
        n++;
        if (n == 4) {
            out[o++] = (uint8_t)(v >> 16);
            out[o++] = (uint8_t)(v >> 8);
            out[o++] = (uint8_t)v;
            v = 0;
            n = 0;
        }
    }
    if (n == 2) {
        v <<= 12;
        out[o++] = (uint8_t)(v >> 16);
    } else if (n == 3) {
        v <<= 6;
        out[o++] = (uint8_t)(v >> 16);
        out[o++] = (uint8_t)(v >> 8);
    }
    *out_len = o;
    return out;
}

/* 截图：Kotlin 返回 JSON {"w":..,"h":..,"base64":..}（base64 为原始 RGBA）→ 解码为缓冲。 */
static uint8_t *op_screenshot(int64_t handle, uint32_t *out_w, uint32_t *out_h) {
    JNIEnv *env = env_get();
    if (env == NULL) return NULL;
    jstring out = (jstring)(*env)->CallStaticObjectMethod(env, g_wb, m_screenshot, (jlong)handle);
    if (out == NULL) return NULL;
    const char *c = (*env)->GetStringUTFChars(env, out, NULL);
    if (c == NULL) { (*env)->DeleteLocalRef(env, out); return NULL; }
    uint32_t w = 0, h = 0;
    uint8_t *rgba = NULL;
    const char *wp = strstr(c, "\"w\":");
    const char *hp = strstr(c, "\"h\":");
    const char *b64 = strstr(c, "\"base64\":\"");
    if (wp && hp && b64) {
        w = (uint32_t)atoi(wp + 4);
        h = (uint32_t)atoi(hp + 4);
        b64 += 10;
        const char *b64end = strchr(b64, '"');
        size_t b64len = b64end ? (size_t)(b64end - b64) : strlen(b64);
        size_t raw_len = 0;
        uint8_t *raw = b64_decode(b64, b64len, &raw_len);
        if (raw && raw_len == (size_t)w * h * 4) {
            rgba = raw;
        } else {
            free(raw);
        }
    }
    (*env)->ReleaseStringUTFChars(env, out, c);
    (*env)->DeleteLocalRef(env, out);
    *out_w = w;
    *out_h = h;
    return rgba;
}

static void op_screenshot_free(uint8_t *ptr) { free(ptr); }

static int op_set_viewport(int64_t handle, uint32_t w, uint32_t h) {
    JNIEnv *env = env_get();
    if (env == NULL) return -1;
    return (*env)->CallStaticIntMethod(env, g_wb, m_set_viewport, (jlong)handle, (jint)w, (jint)h);
}

static int64_t op_native_view(int64_t handle) {
    JNIEnv *env = env_get();
    if (env == NULL) return -1;
    return (int64_t)(*env)->CallStaticLongMethod(env, g_wb, m_native_view, (jlong)handle);
}

static int op_dispatch(int64_t handle, const char *event_json) {
    JNIEnv *env = env_get();
    if (env == NULL) return -1;
    jstring je = js_new(env, event_json);
    int r = (*env)->CallStaticIntMethod(env, g_wb, m_dispatch, (jlong)handle, je);
    (*env)->DeleteLocalRef(env, je);
    return r;
}

static int op_go_back(int64_t handle) {
    JNIEnv *env = env_get();
    if (env == NULL) return -1;
    return (*env)->CallStaticIntMethod(env, g_wb, m_go_back, (jlong)handle);
}

static int op_go_forward(int64_t handle) {
    JNIEnv *env = env_get();
    if (env == NULL) return -1;
    return (*env)->CallStaticIntMethod(env, g_wb, m_go_forward, (jlong)handle);
}

static void op_free_string(char *ptr) { free(ptr); }

/* ── 注册入口 ──────────────────────────────────────────────────── */

JNIEXPORT jint JNICALL Java_com_fastbrowser_Sdk_nativeRegisterWebViewOps(JNIEnv *env, jclass) {
    if (g_wb == NULL) {
        jclass cls = (*env)->FindClass(env, "com/fastbrowser/WebViewBridge");
        if (cls == NULL) return -1;
        g_wb = (jclass)(*env)->NewGlobalRef(env, cls);
        m_create = (*env)->GetStaticMethodID(env, g_wb, "create", "(Ljava/lang/String;II)J");
        m_destroy = (*env)->GetStaticMethodID(env, g_wb, "destroy", "(J)I");
        m_evaluate = (*env)->GetStaticMethodID(env, g_wb, "evaluate", "(JLjava/lang/String;)Ljava/lang/String;");
        m_navigate = (*env)->GetStaticMethodID(env, g_wb, "navigate", "(JLjava/lang/String;)I");
        m_screenshot = (*env)->GetStaticMethodID(env, g_wb, "screenshot", "(J)Ljava/lang/String;");
        m_set_viewport = (*env)->GetStaticMethodID(env, g_wb, "setViewport", "(JII)I");
        m_native_view = (*env)->GetStaticMethodID(env, g_wb, "nativeView", "(J)J");
        m_dispatch = (*env)->GetStaticMethodID(env, g_wb, "dispatchEvent", "(JLjava/lang/String;)I");
        m_go_back = (*env)->GetStaticMethodID(env, g_wb, "goBack", "(J)I");
        m_go_forward = (*env)->GetStaticMethodID(env, g_wb, "goForward", "(J)I");
        if (g_vm == NULL) (*env)->GetJavaVM(env, &g_vm);
    }

    static FbWebViewOps ops;
    memset(&ops, 0, sizeof(ops));
    ops.create = op_create;
    ops.destroy = op_destroy;
    ops.evaluate = op_evaluate;
    ops.navigate = op_navigate;
    ops.screenshot = op_screenshot;
    ops.screenshot_free = op_screenshot_free;
    ops.set_viewport = op_set_viewport;
    ops.native_view = op_native_view;
    ops.dispatch_event = op_dispatch;
    ops.go_back = op_go_back;
    ops.go_forward = op_go_forward;
    ops.free_string = op_free_string;

    return fastbrowser_register_webview_ops(&ops);
}

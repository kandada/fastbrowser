/*
 * Copyright (c) 2025 xiefujin <490021684@qq.com>
 * Licensed under Apache-2.0, see LICENSE file for full license terms.
 *
 * jni_glue.c — Android JNI glue (方案 B: staticlib + NDK CMake).
 *
 *   Rust staticlib (libfastbrowser.a)  →  extern "C" fastbrowser_* symbols
 *   jni_glue.c (this file)            →  Java_com_fastbrowser_Sdk_native*
 *   Kotlin com.fastbrowser.Sdk        →  System.loadLibrary("fastbrowser_jni")
 *
 * Event/frame callbacks are forwarded to Kotlin static methods
 * Sdk.onPageEvent(int tab, String json) / Sdk.onViewFrame(int tab, String json).
 */

#include <jni.h>
#include <stdint.h>
#include <string.h>

#include "fastbrowser.h"

static JavaVM *g_vm = NULL;
static jclass g_sdk_class = NULL;
static jmethodID g_on_event = NULL;
static jmethodID g_on_frame = NULL;

static jstring js(JNIEnv *env, char *rust_string) {
    if (rust_string == NULL) {
        return (*env)->NewStringUTF(env, "{\"error\":{\"kind\":\"internal\",\"message\":\"null result\"}}");
    }
    jstring out = (*env)->NewStringUTF(env, rust_string);
    fastbrowser_free_string(rust_string);
    return out;
}

static void call_static_void(JNIEnv *env, jmethodID m, jint tab, const char *json) {
    if (m == NULL) return;
    jstring s = (*env)->NewStringUTF(env, json);
    (*env)->CallStaticVoidMethod(env, g_sdk_class, m, tab, s);
    (*env)->DeleteLocalRef(env, s);
}

static void event_cb(uint32_t tab, const char *json) {
    if (g_vm == NULL || g_sdk_class == NULL) return;
    JNIEnv *env = NULL;
    jint attached = (*g_vm)->GetEnv(g_vm, (void **)&env, JNI_VERSION_1_6);
    if (attached == JNI_EDETACHED) {
        if ((*g_vm)->AttachCurrentThread(g_vm, (JNIEnv **)&env, NULL) != JNI_OK) return;
    }
    if (env != NULL && attached == JNI_OK) {
        call_static_void(env, g_on_event, (jint)tab, json);
    }
}

static void frame_cb(uint32_t tab, const char *json) {
    if (g_vm == NULL || g_sdk_class == NULL) return;
    JNIEnv *env = NULL;
    jint attached = (*g_vm)->GetEnv(g_vm, (void **)&env, JNI_VERSION_1_6);
    if (attached == JNI_EDETACHED) {
        if ((*g_vm)->AttachCurrentThread(g_vm, (JNIEnv **)&env, NULL) != JNI_OK) return;
    }
    if (env != NULL && attached == JNI_OK) {
        call_static_void(env, g_on_frame, (jint)tab, json);
    }
}

JNIEXPORT jint JNICALL JNI_OnLoad(JavaVM *vm, void *reserved) {
    (void)reserved;
    g_vm = vm;
    JNIEnv *env = NULL;
    if ((*vm)->GetEnv(vm, (void **)&env, JNI_VERSION_1_6) != JNI_OK) return JNI_ERR;
    jclass cls = (*env)->FindClass(env, "com/fastbrowser/Sdk");
    if (cls == NULL) return JNI_ERR;
    g_sdk_class = (jclass)(*env)->NewGlobalRef(env, cls);
    g_on_event = (*env)->GetStaticMethodID(env, cls, "onPageEvent", "(ILjava/lang/String;)V");
    g_on_frame = (*env)->GetStaticMethodID(env, cls, "onViewFrame", "(ILjava/lang/String;)V");
    return JNI_VERSION_1_6;
}

/* ── native methods ─────────────────────────────────────────── */

JNIEXPORT jstring JNICALL Java_com_fastbrowser_Sdk_nativeVersion(JNIEnv *env, jclass) {
    char *v = fastbrowser_version();
    jstring out = (*env)->NewStringUTF(env, v);
    fastbrowser_free_string(v);
    return out;
}

JNIEXPORT jstring JNICALL Java_com_fastbrowser_Sdk_nativeInit(JNIEnv *env, jclass, jstring config_json) {
    const char *cfg = (*env)->GetStringUTFChars(env, config_json, NULL);
    if (cfg == NULL) return js(env, fastbrowser_init("{}"));
    fastbrowser_register_event_callback(event_cb);
    fastbrowser_register_viewframe_callback(frame_cb);
    char *result = fastbrowser_init(cfg);
    (*env)->ReleaseStringUTFChars(env, config_json, cfg);
    return js(env, result);
}

JNIEXPORT jstring JNICALL Java_com_fastbrowser_Sdk_nativeShutdown(JNIEnv *env, jclass) {
    return js(env, fastbrowser_shutdown());
}

JNIEXPORT jstring JNICALL Java_com_fastbrowser_Sdk_nativeOpen(JNIEnv *env, jclass, jstring url) {
    const char *u = (*env)->GetStringUTFChars(env, url, NULL);
    char *result = fastbrowser_open(u);
    (*env)->ReleaseStringUTFChars(env, url, u);
    return js(env, result);
}

JNIEXPORT jstring JNICALL Java_com_fastbrowser_Sdk_nativeNavigate(JNIEnv *env, jclass, jstring url) {
    const char *u = (*env)->GetStringUTFChars(env, url, NULL);
    char *result = fastbrowser_navigate(u);
    (*env)->ReleaseStringUTFChars(env, url, u);
    return js(env, result);
}

JNIEXPORT jstring JNICALL Java_com_fastbrowser_Sdk_nativeToolList(JNIEnv *env, jclass) {
    return js(env, fastbrowser_tool_list());
}

JNIEXPORT jstring JNICALL Java_com_fastbrowser_Sdk_nativeToolCall(JNIEnv *env, jclass, jstring name, jstring params_json) {
    const char *n = (*env)->GetStringUTFChars(env, name, NULL);
    const char *p = (*env)->GetStringUTFChars(env, params_json, NULL);
    char *result = fastbrowser_tool_call(n, p);
    (*env)->ReleaseStringUTFChars(env, name, n);
    (*env)->ReleaseStringUTFChars(env, params_json, p);
    return js(env, result);
}

JNIEXPORT jstring JNICALL Java_com_fastbrowser_Sdk_nativeSnapshot(JNIEnv *env, jclass) {
    return js(env, fastbrowser_snapshot());
}

JNIEXPORT jstring JNICALL Java_com_fastbrowser_Sdk_nativeScreenshot(JNIEnv *env, jclass) {
    return js(env, fastbrowser_screenshot());
}

JNIEXPORT jstring JNICALL Java_com_fastbrowser_Sdk_nativeGetView(JNIEnv *env, jclass) {
    return js(env, fastbrowser_get_view());
}

JNIEXPORT jstring JNICALL Java_com_fastbrowser_Sdk_nativeSetViewport(JNIEnv *env, jclass, jint width, jint height) {
    return js(env, fastbrowser_set_viewport((uint32_t)width, (uint32_t)height));
}

JNIEXPORT jstring JNICALL Java_com_fastbrowser_Sdk_nativeGetInfo(JNIEnv *env, jclass) {
    return js(env, fastbrowser_get_info());
}

JNIEXPORT jstring JNICALL Java_com_fastbrowser_Sdk_nativeStatus(JNIEnv *env, jclass) {
    return js(env, fastbrowser_status());
}

JNIEXPORT void JNICALL Java_com_fastbrowser_Sdk_nativeSetPermission(JNIEnv *env, jclass, jstring resource, jboolean allowed) {
    const char *r = (*env)->GetStringUTFChars(env, resource, NULL);
    fastbrowser_set_permission(r, allowed ? 1 : 0);
    (*env)->ReleaseStringUTFChars(env, resource, r);
}

JNIEXPORT void JNICALL Java_com_fastbrowser_Sdk_nativeFreeString(JNIEnv *env, jclass, jstring /*unused*/) {
    (void)env;
    /* 字符串已在 js() 中释放；保留此入口用于兼容。 */
}

package com.fastbrowser

/**
 * fastbrowser JNI 封装（对齐 fastbrowser_c/src/jni_glue.c 的 native 方法）。
 * 全部进出为 JSON 字符串；错误形如 {"error":{"kind":"...","message":"..."}}。
 */
object Sdk {
    init { System.loadLibrary("fastbrowser_jni") }

    private external fun nativeVersion(): String
    private external fun nativeInit(configJson: String): String
    private external fun nativeShutdown(): String
    private external fun nativeOpen(url: String): String
    private external fun nativeNavigate(url: String): String
    private external fun nativeToolList(): String
    private external fun nativeToolCall(name: String, paramsJson: String): String
    private external fun nativeSnapshot(): String
    private external fun nativeScreenshot(): String
    private external fun nativeGetView(): String
    private external fun nativeSetViewport(width: Int, height: Int): String
    private external fun nativeGetInfo(): String
    private external fun nativeStatus(): String
    private external fun nativeSetPermission(resource: String, allowed: Boolean)
    private external fun nativeRegisterWebViewOps(): Int

    // ── 供 C 回调（jni_glue.c）调用 ─────────────────────────────
    @JvmStatic fun onPageEvent(tab: Int, json: String) { /* 宿主可在此展示事件 */ }
    @JvmStatic fun onViewFrame(tab: Int, json: String) { /* 宿主可在此渲染预览帧 */ }

    val version: String get() = nativeVersion()
    fun registerWebViewOps(): Int = nativeRegisterWebViewOps()
    fun init(configJson: String): String = nativeInit(configJson)
    fun shutdown(): String = nativeShutdown()
    fun open(url: String): String = nativeOpen(url)
    fun navigate(url: String): String = nativeNavigate(url)
    fun toolList(): String = nativeToolList()
    fun toolCall(name: String, paramsJson: String): String = nativeToolCall(name, paramsJson)
    fun snapshot(): String = nativeSnapshot()
    fun screenshot(): String = nativeScreenshot()
    fun status(): String = nativeStatus()
    fun setPermission(resource: String, allowed: Boolean) = nativeSetPermission(resource, allowed)
}

package com.fastbrowser

import org.json.JSONObject

/**
 * fastbrowser 内核的 Android 绑定（App 无关，可复用）。
 *
 * 配合 fastbrowser_c（JNI 胶水 + libfastbrowser_jni.so）使用：
 *   System.loadLibrary("fastbrowser_jni")
 *   val fb = Sdk
 *   Sdk.init(JSONObject().put("engine", "webview"))
 *
 * 事件/帧回调由 JNI 胶水转发到本类的静态方法 onPageEvent / onViewFrame。
 */
object Sdk {

    init {
        System.loadLibrary("fastbrowser_jni")
    }

    // ── JNI 原生方法（fastbrowser_c/src/jni_glue.c）───────────────

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
    private external fun nativeFreeString(s: String)

    // ── 宿主回调（JNI 胶水调用）───────────────────────────────────

    @JvmStatic
    fun onPageEvent(tab: Int, eventJson: String) {
        eventListener?.invoke(tab, JSONObject(eventJson))
    }

    @JvmStatic
    fun onViewFrame(tab: Int, frameJson: String) {
        frameListener?.invoke(tab, JSONObject(frameJson))
    }

    @Volatile var eventListener: ((Int, JSONObject) -> Unit)? = null
    @Volatile var frameListener: ((Int, JSONObject) -> Unit)? = null

    // ── 公开 API ─────────────────────────────────────────────────

    val version: String get() = nativeVersion()

    @Throws(Exception::class)
    fun init(engine: String = "mock", config: JSONObject = JSONObject()) {
        config.put("engine", engine)
        val result = JSONObject(nativeInit(config.toString()))
        raiseIfError(result)
    }

    fun open(url: String): JSONObject = json(nativeOpen(url))
    fun navigate(url: String): JSONObject = json(nativeNavigate(url))
    fun toolList(): JSONArray = JSONArray(nativeToolList())
    fun toolCall(name: String, params: JSONObject = JSONObject()): JSONObject =
        json(nativeToolCall(name, params.toString()))
    fun snapshot(): JSONObject = json(nativeSnapshot())
    fun screenshot(): JSONObject = json(nativeScreenshot())
    fun getView(): JSONObject = json(nativeGetView())
    fun setViewport(width: Int, height: Int): JSONObject = json(nativeSetViewport(width, height))
    fun getInfo(): JSONObject = json(nativeGetInfo())
    fun status(): JSONObject = json(nativeStatus())
    fun setPermission(resource: String, allowed: Boolean) =
        nativeSetPermission(resource, allowed)
    fun shutdown(): JSONObject = json(nativeShutdown())

    private fun json(s: String): JSONObject {
        val obj = JSONObject(s)
        raiseIfError(obj)
        return obj
    }

    private fun raiseIfError(obj: JSONObject) {
        if (obj.has("error")) {
            val e = obj.getJSONObject("error")
            throw FastBrowserException(
                e.optString("message", "unknown"),
                e.optString("kind", "unknown")
            )
        }
    }
}

class FastBrowserException(message: String, val kind: String) : Exception(message)

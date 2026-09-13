package com.fastbrowser

import android.graphics.Bitmap
import android.util.Base64
import android.webkit.WebView
import android.webkit.WebViewClient
import java.util.concurrent.atomic.AtomicLong

/**
 * WebViewOps 宿主实现（供 jni_webview_bridge.c 的 FbWebViewOps 回调调用）。
 * 真实 Android WebView = Chromium；Rust 内核经此桥驱动页面。
 * 必须由 UI 线程持有 WebView（evaluate/navigate 等切到主线程执行）。
 */
object WebViewBridge {
    private val views = HashMap<Long, WebView>()
    private val nextHandle = AtomicLong(1)

    @JvmStatic fun create(url: String, width: Int, height: Int): Long {
        val h = nextHandle.getAndIncrement()
        MainHandler.post {
            val wv = WebView(MainHandler.context!!)
            wv.settings.javaScriptEnabled = true
            wv.settings.domStorageEnabled = true
            wv.webViewClient = WebViewClient()
            wv.layout(0, 0, width, height)
            wv.loadUrl(url)
            views[h] = wv
        }
        return h
    }

    @JvmStatic fun destroy(handle: Long): Int {
        MainHandler.post { views.remove(handle)?.destroy() }
        return 0
    }

    @JvmStatic fun evaluate(handle: Long, script: String): String? {
        var result: String? = "null"
        MainHandler.sync {
            val wv = views[handle] ?: return@sync
            wv.evaluateJavascript("(function(){try{return JSON.stringify(eval(${js(script)}));}catch(e){return JSON.stringify({error:String(e)});}})()") { r ->
                result = r ?: "null"
            }
        }
        return result
    }

    @JvmStatic fun navigate(handle: Long, url: String): Int {
        MainHandler.post { views[handle]?.loadUrl(url) }
        return 0
    }

    @JvmStatic fun screenshot(handle: Long): String {
        var out = """{"w":0,"h":0,"base64":""}"""
        MainHandler.sync {
            val wv = views[handle] ?: return@sync
            val bmp = wv.capturePicture() // 或 draw 到 Canvas
            out = bitmapToRgbaJson(bmp)
        }
        return out
    }

    @JvmStatic fun setViewport(handle: Long, width: Int, height: Int): Int {
        MainHandler.post {
            views[handle]?.layout(0, 0, width, height)
        }
        return 0
    }

    @JvmStatic fun nativeView(handle: Long): Long = handle // 返回 WebView 句柄本身

    @JvmStatic fun dispatchEvent(handle: Long, eventJson: String): Int {
        // 移动端由 JS 派发（capabilities.supports_coordinate_input=false 时内核自动降级）
        return 0
    }

    @JvmStatic fun goBack(handle: Long): Int {
        MainHandler.post { views[handle]?.goBack() }
        return 0
    }

    @JvmStatic fun goForward(handle: Long): Int {
        MainHandler.post { views[handle]?.goForward() }
        return 0
    }

    // ── 工具 ────────────────────────────────────────────────────
    private fun js(s: String) = "\"" + s.replace("\\", "\\\\").replace("\"", "\\\"") + "\""

    private fun bitmapToRgbaJson(bmp: Bitmap): String {
        val w = bmp.width
        val h = bmp.height
        val pixels = IntArray(w * h)
        bmp.getPixels(pixels, 0, w, 0, 0, w, h)
        val rgba = ByteArray(w * h * 4)
        for (i in pixels.indices) {
            val p = pixels[i]
            rgba[i * 4] = ((p shr 16) and 0xFF).toByte()
            rgba[i * 4 + 1] = ((p shr 8) and 0xFF).toByte()
            rgba[i * 4 + 2] = (p and 0xFF).toByte()
            rgba[i * 4 + 3] = ((p shr 24) and 0xFF).toByte()
        }
        val b64 = Base64.encodeToString(rgba, Base64.NO_WRAP)
        return """{"w":$w,"h":$h,"base64":"$b64"}"""
    }
}

/** 简单的 UI 线程助手（可换成 Handler/Looper 或协程）。 */
object MainHandler {
    var context: android.content.Context? = null

    fun post(block: () -> Unit) {
        android.os.Handler(android.os.Looper.getMainLooper()).post(block)
    }

    /** 阻塞等待主线程执行（evaluate 需要同步返回值）。 */
    fun sync(block: () -> Unit) {
        val latch = java.util.concurrent.CountDownLatch(1)
        post {
            try { block() } finally { latch.countDown() }
        }
        latch.await()
    }
}

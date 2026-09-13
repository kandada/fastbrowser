package com.fastbrowser.device

import android.app.Activity
import android.os.Bundle
import android.widget.Button
import android.widget.ScrollView
import android.widget.TextView
import com.fastbrowser.MainHandler
import com.fastbrowser.Sdk
import com.fastbrowser.WebViewBridge
import org.json.JSONArray
import java.io.File

/**
 * 真机测试宿主入口：注册 WebViewOps → init(webview) → 执行 cases → 写报告。
 */
class MainActivity : Activity() {
    private lateinit var log: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        MainHandler.context = applicationContext

        val tv = TextView(this)
        val btn = Button(this).apply { text = "运行真机用例" }
        val root = android.widget.LinearLayout(this).apply {
            orientation = android.widget.LinearLayout.VERTICAL
            addView(btn)
            val sv = ScrollView(this@MainActivity).apply { addView(tv) }
            addView(sv, android.widget.LinearLayout.LayoutParams.MATCH_PARENT, 0)
        }
        setContentView(root)
        log = tv

        btn.setOnClickListener {
            Thread {
                runCases()
            }.start()
        }
        log.text = "点击“运行真机用例”开始\n"
    }

    private fun runCases() {
        append("内核版本: ${Sdk.version}")
        val reg = Sdk.registerWebViewOps()
        append("registerWebViewOps: $reg")
        val init = Sdk.init("""{"engine":"webview"}""")
        append("init: $init")

        // 加载 assets/cases/*.json（gradle 已把 shared/cases 同步进 assets）
        val cases = ArrayList<String>()
        for (f in assets.list("cases") ?: emptyArray()) {
            if (f.endsWith(".json")) cases.add(assets.open("cases/$f").bufferedReader().use { it.readText() })
        }
        append("加载用例: ${cases.size}")

        val report = CaseRunner.run(cases)
        append("用例完成: ${report.length()}")
        val outDir = File(getExternalFilesDir(null), "fb-realdevice")
        outDir.mkdirs()
        val outFile = File(outDir, "android.json")
        outFile.writeText(report.toString(2))
        append("报告: ${outFile.absolutePath}")
    }

    private fun append(s: String) {
        runOnUiThread { log.text = "${log.text}\n$s" }
    }
}

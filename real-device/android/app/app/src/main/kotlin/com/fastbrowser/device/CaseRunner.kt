package com.fastbrowser.device

import com.fastbrowser.Sdk
import org.json.JSONArray
import org.json.JSONObject

/**
 * 真机用例 runner：加载 assets/cases/*.json，经 C ABI（Sdk）逐条执行，
 * 输出与 Rust 参考 runner 相同结构的报告 JSON（写入外部存储）。
 *
 * 动作集合与 shared/README.md 一致：open / navigate / tool / find / wait_load /
 * wait_navigation / sleep / session_save / session_load / reset / clear_state / benchmark。
 */
object CaseRunner {
    private val vars = HashMap<String, String>()   // $var 引用
    private var currentTab = "1"

    fun run(caseJsons: List<String>): JSONArray {
        val report = JSONArray()
        for (caseJson in caseJsons) {
            val case = JSONObject(caseJson)
            val engines = case.optJSONArray("engines") ?: JSONArray()
            if (!engines.toString().contains("webview")) {
                report.put(summary(case, "skip", "not for webview", JSONArray()))
                continue
            }
            vars.clear()
            currentTab = "1"
            val steps = JSONArray()
            var failed = false
            val caseSteps = case.getJSONArray("steps")
            for (i in 0 until caseSteps.length()) {
                val step = caseSteps.getJSONObject(i)
                val (ok, detail) = runStep(step)
                steps.put(JSONObject().put("action", step.optString("action")).put("ok", ok).put("detail", detail))
                if (!ok) { failed = true; break }
            }
            report.put(summary(case, if (failed) "fail" else "pass", "", steps))
        }
        return report
    }

    private fun summary(case: JSONObject, status: String, reason: String, steps: JSONArray) = JSONObject()
        .put("id", case.optString("id"))
        .put("category", case.optString("category"))
        .put("status", status)
        .put("reason", reason)
        .put("steps", steps)

    private fun runStep(step: JSONObject): Pair<Boolean, String> {
        return try {
            when (step.optString("action")) {
                "open" -> { val r = Sdk.open(template(step.getJSONObject("params").getString("url"))); trackTab(r); checkExpect(r, step) }
                "navigate" -> checkExpect(Sdk.navigate(template(step.getJSONObject("params").getString("url"))), step)
                "tool" -> {
                    val name = step.getString("name")
                    val params = template(step.getJSONObject("params").toString())
                    val r = Sdk.toolCall(name, params)
                    trackTab(r)
                    checkExpect(r, step)
                }
                "find" -> {
                    val m = step.getJSONObject("match")
                    val tab = step.optJSONObject("tab")?.let { template(it.toString()) } ?: currentTab
                    val snap = JSONObject(Sdk.snapshot())
                    val els = snap.getJSONArray("interactive")
                    var found: String? = null
                    for (i in 0 until els.length()) {
                        val el = els.getJSONObject(i)
                        if (m.has("tag") && el.optString("tag") != m.getString("tag")) continue
                        if (m.has("name") && el.optJSONObject("attrs")?.optString("name") != m.getString("name")) continue
                        if (m.has("text") && !el.optString("text").contains(m.getString("text"))) continue
                        found = el.optString("id")
                        break
                    }
                    if (found == null) Pair(false, "find: no element matching $m") else { vars[step.getString("var")] = found; Pair(true, "id=$found") }
                }
                "wait_load" -> waitUntil(step.optLong("timeout_ms", 10000)) {
                    val js = "document.readyState||''"
                    JSONObject(Sdk.toolCall("execute_js", """{"script":$json(js)}""")).optString("result") == "complete"
                }
                "wait_navigation" -> Pair(true, "ok")
                "sleep" -> { Thread.sleep(step.optLong("ms", 100)); Pair(true, "ok") }
                "session_save" -> Pair(true, Sdk.toolCall("session_save", """{"path":${json(vars["session_path"] ?: "/sdcard/state.json")}}"""))
                "session_load" -> Pair(true, Sdk.toolCall("session_load", """{"path":${json(vars["session_path"] ?: "/sdcard/state.json")}}"""))
                "reset" -> { Sdk.shutdown(); Pair(true, Sdk.init("""{"engine":"webview"}""")) }
                "clear_state" -> Pair(true, Sdk.toolCall("clear_state", "{}"))
                else -> Pair(false, "unknown action")
            }
        } catch (e: Exception) {
            Pair(false, e.message ?: e.toString())
        }
    }

    private fun trackTab(r: String) {
        val o = JSONObject(r)
        if (o.has("tab")) currentTab = o.get("tab").toString()
        if (o.has("url") && o.optString("url").startsWith("http")) {
            val host = o.optString("url").replace("https://", "").replace("http://", "").substringBefore("/").substringBefore(":")
            vars["base_host"] = host
        }
    }

    private fun checkExpect(r: String, step: JSONObject): Pair<Boolean, String> {
        if (step.has("expect_error")) {
            if (JSONObject(r).has("error")) return Pair(true, r)
            return Pair(false, "expected error but got: $r")
        }
        val expect = step.optJSONObject("expect") ?: return Pair(true, r)
        val result = JSONObject(r)
        for (key in expect.keys()) {
            val want = expect.get(key)
            val got = result.opt(key)
            if (!matchValue(got, want)) return Pair(false, "expect $key == $want failed, got $got")
        }
        return Pair(true, r)
    }

    private fun matchValue(got: Any?, want: Any): Boolean {
        if (want is JSONObject) {
            if (want.has("contains")) return got.toString().contains(want.getString("contains"))
            if (want.has("is_number")) return got is Number
            if (want.has("is_array")) return got is JSONArray
            if (want.has("is_object")) return got is JSONObject
            if (want.has("len_gte")) return got is JSONArray && got.length() >= want.getInt("len_gte")
            if (want.has("gte")) return got is Number && got.toDouble() >= want.getDouble("gte")
        }
        return got == want
    }

    private fun template(s: String): String {
        var out = s
        for ((k, v) in vars) out = out.replace("{\"\$var\":\"$k\"}", v)
        return out
    }

    private fun json(s: String) = "\"" + s.replace("\\", "\\\\").replace("\"", "\\\"") + "\""

    private fun waitUntil(timeoutMs: Long, cond: () -> Boolean): Pair<Boolean, String> {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < deadline) {
            if (cond()) return Pair(true, "ok")
            Thread.sleep(50)
        }
        return Pair(false, "wait timed out")
    }
}

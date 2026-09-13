import Foundation
import Cfastbrowser

/// fastbrowser C ABI 封装（Swift）。进出为 JSON 字符串；错误形如 {"error":{...}}。
enum Fastbrowser {
    static func registerWebViewOps() {
        var ops = FbWebViewOps()
        ops.create = fbwv_create
        ops.destroy = fbwv_destroy
        ops.evaluate = fbwv_evaluate
        ops.navigate = fbwv_navigate
        ops.screenshot = fbwv_screenshot
        ops.screenshot_free = fbwv_screenshot_free
        ops.set_viewport = fbwv_set_viewport
        ops.native_view = fbwv_native_view
        ops.dispatch_event = fbwv_dispatch_event
        ops.go_back = fbwv_go_back
        ops.go_forward = fbwv_go_forward
        ops.free_string = fbwv_free_string
        fastbrowser_register_webview_ops(&ops)
    }

    static func call(_ f: @escaping (UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>?, _ arg: String? = nil) -> String {
        var out: UnsafeMutablePointer<CChar>? = nil
        arg.withCString { c in
            out = f(c)
        }
        defer { if let o = out { fastbrowser_free_string(o) } }
        return out.map { String(cString: $0) } ?? #"{"error":{"message":"null"}}"#
    }

    static func initKernel(_ configJson: String) -> String {
        registerWebViewOps()
        var out: UnsafeMutablePointer<CChar>? = nil
        configJson.withCString { out = fastbrowser_init($0) }
        defer { if let o = out { fastbrowser_free_string(o) } }
        return out.map { String(cString: $0) } ?? "null"
    }
    static func open(_ url: String) -> String { url.withCString { String(cString: fastbrowser_open($0)!) } }
    static func toolCall(_ name: String, _ params: String) -> String {
        var out: UnsafeMutablePointer<CChar>? = nil
        name.withCString { n in
            params.withCString { p in out = fastbrowser_tool_call(n, p) }
        }
        defer { if let o = out { fastbrowser_free_string(o) } }
        return out.map { String(cString: $0) } ?? "null"
    }
    static func shutdown() { fastbrowser_shutdown() }
}

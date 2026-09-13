import Foundation
import WebKit

/// WKWebView 宿主（iOS = WebKit）。实现 FbWebViewOps 回调，经 C ABI 注入内核。
/// 内核不直接持有 WKWebView；全部操作经 evaluateJavaScript / load 驱动。
final class WebViewBridge {
    static let shared = WebViewBridge()
    private var views: [Int64: WKWebView] = [:]
    private var nextHandle: Int64 = 1
    private let lock = NSLock()

    func create(url: String, width: UInt32, height: UInt32) -> Int64 {
        let handle = nextHandle; nextHandle += 1
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .default()
        let wv = WKWebView(frame: CGRect(x: 0, y: 0, width: CGFloat(width), height: CGFloat(height)), configuration: config)
        wv.load(URLRequest(url: URL(string: url) ?? URL(string: "about:blank")!))
        lock.lock(); views[handle] = wv; lock.unlock()
        return handle
    }

    func destroy(_ h: Int64) { lock.lock(); views.removeValue(forKey: h); lock.unlock() }

    /// evaluateJavaScript 返回 JSON 字符串（同步：用信号量等待回调）。
    func evaluate(_ h: Int64, _ script: String) -> String? {
        guard let wv = views[h] else { return "null" }
        var out: String = "null"
        let sem = DispatchSemaphore(value: 0)
        let js = "JSON.stringify((function(){try{return eval(\(json(script)));}catch(e){return {error:String(e)};}})())"
        DispatchQueue.main.async {
            wv.evaluateJavaScript(js) { r, _ in
                if let s = r as? String { out = s }
                sem.signal()
            }
        }
        _ = sem.wait(timeout: .now() + 10)
        return out
    }

    func navigate(_ h: Int64, _ url: String) {
        guard let wv = views[h] else { return }
        DispatchQueue.main.async { wv.load(URLRequest(url: URL(string: url) ?? URL(string: "about:blank")!)) }
    }

    /// 截图：captureSnapshot 的 PNG → base64 → {"w","h","base64"}（内核解码为 RGBA）。
    func screenshot(_ h: Int64) -> String {
        guard let wv = views[h] else { return #"{"w":0,"h":0,"base64":""}"# }
        var out = #"{"w":0,"h":0,"base64":""}"#
        let sem = DispatchSemaphore(value: 0)
        DispatchQueue.main.async {
            wv.takeSnapshot(with: nil) { img, _ in
                if let cg = img?.cgImage {
                    let w = cg.width, h = cg.height
                    var rgba = [UInt8](repeating: 0, count: w * h * 4)
                    if let ctx = CGContext(data: &rgba, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4,
                                          space: CGColorSpaceCreateDeviceRGB(),
                                          bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) {
                        ctx.draw(cg, in: CGRect(x: 0, y: 0, width: w, height: h))
                    }
                    out = #"{"w":"# + "\(w)" + #","h":"# + "\(h)" + #","base64":"# + Data(rgba).base64EncodedString() + #""}"#
                }
                sem.signal()
            }
        }
        _ = sem.wait(timeout: .now() + 10)
        return out
    }

    private func json(_ s: String) -> String {
        let data = try! JSONSerialization.data(withJSONObject: s, options: [])
        return String(data: data, encoding: .utf8)!
    }
}

// ── FbWebViewOps C 回调（@_cdecl 全局函数）────────────────────────
@_cdecl("fbwv_create")
func fbwv_create(url: UnsafePointer<CChar>?, width: UInt32, height: UInt32) -> Int64 {
    let u = url.map { String(cString: $0) } ?? "about:blank"
    return WebViewBridge.shared.create(url: u, width: width, height: height)
}
@_cdecl("fbwv_destroy")
func fbwv_destroy(h: Int64) -> Int32 { WebViewBridge.shared.destroy(h); return 0 }
@_cdecl("fbwv_evaluate")
func fbwv_evaluate(h: Int64, script: UnsafePointer<CChar>?) -> UnsafeMutablePointer<CChar>? {
    let s = script.map { String(cString: $0) } ?? ""
    let r = WebViewBridge.shared.evaluate(h, s) ?? "null"
    return strdup(r)
}
@_cdecl("fbwv_navigate")
func fbwv_navigate(h: Int64, url: UnsafePointer<CChar>?) -> Int32 {
    let u = url.map { String(cString: $0) } ?? "about:blank"
    WebViewBridge.shared.navigate(h, u); return 0
}
@_cdecl("fbwv_screenshot")
func fbwv_screenshot(h: Int64, outW: UnsafeMutablePointer<UInt32>, outH: UnsafeMutablePointer<UInt32>) -> UnsafeMutablePointer<UInt8>? {
    let j = WebViewBridge.shared.screenshot(h)
    guard let d = j.data(using: .utf8),
          let o = try? JSONSerialization.jsonObject(with: d) as? [String: Any],
          let w = o["w"] as? Int, let hh = o["h"] as? Int,
          let b64 = o["base64"] as? String,
          let raw = Data(base64Encoded: b64) else { return nil }
    outW.pointee = UInt32(w); outH.pointee = UInt32(hh)
    let buf = UnsafeMutablePointer<UInt8>.allocate(capacity: raw.count)
    raw.copyBytes(to: buf, count: raw.count)
    return buf
}
@_cdecl("fbwv_screenshot_free")
func fbwv_screenshot_free(ptr: UnsafeMutablePointer<UInt8>?) { ptr?.deallocate() }
@_cdecl("fbwv_set_viewport")
func fbwv_set_viewport(h: Int64, w: UInt32, hh: UInt32) -> Int32 { 0 }
@_cdecl("fbwv_native_view")
func fbwv_native_view(h: Int64) -> Int64 { h }
@_cdecl("fbwv_dispatch_event")
func fbwv_dispatch_event(h: Int64, ev: UnsafePointer<CChar>?) -> Int32 { 0 }
@_cdecl("fbwv_go_back")
func fbwv_go_back(h: Int64) -> Int32 { 0 }
@_cdecl("fbwv_go_forward")
func fbwv_go_forward(h: Int64) -> Int32 { 0 }
@_cdecl("fbwv_free_string")
func fbwv_free_string(ptr: UnsafeMutablePointer<CChar>?) { ptr?.deallocate() }

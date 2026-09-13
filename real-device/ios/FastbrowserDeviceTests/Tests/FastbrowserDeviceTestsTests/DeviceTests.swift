import XCTest
@testable import FastbrowserDeviceTests

/// iOS 真机用例（XCTest 宿主）。加载 shared/cases 需要把 JSON 加入测试 bundle 资源。
/// 运行：xcodebuild test -scheme FastbrowserDeviceTests -destination 'platform=iOS,name=<真机>'
final class DeviceTests: XCTestCase {

    func testSmokeOpen() {
        Fastbrowser.registerWebViewOps()
        let initR = Fastbrowser.initKernel(#"{"engine":"webview"}"#)
        XCTAssertTrue(initR.contains("\"ok\""), "init: \(initR)")

        let openR = Fastbrowser.open("https://example.com")
        XCTAssertTrue(openR.contains("\"tab\""), "open: \(openR)")

        // 快照应含交互元素
        var ok = false
        for _ in 0..<50 {
            let snap = Fastbrowser.toolCall("snapshot", "{}")
            if snap.contains("\"interactive\"") { ok = true; break }
            Thread.sleep(forTimeInterval: 0.1)
        }
        XCTAssertTrue(ok, "snapshot should contain interactive elements")
        Fastbrowser.shutdown()
    }

    func testToolTypeClick() {
        Fastbrowser.initKernel(#"{"engine":"webview"}"#)
        Fastbrowser.open("data:text/html,<html><body><input name='q'><button onclick='document.body.dataset.ok=1'>Go</button></body></html>")
        Thread.sleep(forTimeInterval: 1)
        // find input via snapshot 后 type
        let snap = Fastbrowser.toolCall("snapshot", "{}")
        let id = extractFirstId(snap)
        XCTAssertNotNil(id, "should find an element: \(snap)")
        let _ = Fastbrowser.toolCall("type", #"{"id":"\#(id ?? "a")","text":"hello"}"#)
        let v = Fastbrowser.toolCall("execute_js", #"{"script":"document.querySelector('input').value"}"#)
        XCTAssertTrue(v.contains("hello"), "typed value: \(v)")
        Fastbrowser.shutdown()
    }

    private func extractFirstId(_ snap: String) -> String? {
        guard let d = snap.data(using: .utf8),
              let o = try? JSONSerialization.jsonObject(with: d) as? [String: Any],
              let els = o["interactive"] as? [[String: Any]] else { return nil }
        return els.first?["id"] as? String
    }
}

import XCTest
@testable import Fastbrowser

final class FastbrowserTests: XCTestCase {
    func testVersion() {
        XCTAssertFalse(FastBrowser.version().isEmpty)
    }

    func testLifecycle() throws {
        let fb = try FastBrowser(engine: "mock")
        let out = try fb.open("https://example.com")
        XCTAssertEqual(out["title"] as? String, "Example Page")

        let tools = try fb.toolList()
        XCTAssertGreaterThanOrEqual(tools.count, 30)

        let links = try fb.toolCall("extract_links")
        XCTAssertGreaterThanOrEqual((links["links"] as? [Any])?.count ?? 0, 1)

        let info = try fb.info()
        XCTAssertEqual(info["engine"] as? String, "mock")

        try fb.shutdown()
    }

    func testUnknownToolThrows() throws {
        let fb = try FastBrowser(engine: "mock")
        XCTAssertThrowsError(try fb.toolCall("does_not_exist")) { err in
            XCTAssertEqual((err as? FastbrowserError)?.kind, "invalid_argument")
        }
        try fb.shutdown()
    }
}

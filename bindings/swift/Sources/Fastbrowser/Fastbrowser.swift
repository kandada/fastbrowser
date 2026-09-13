import Cfastbrowser
import Foundation

/// fastbrowser 内核错误。
public struct FastbrowserError: Error, CustomStringConvertible {
    public let message: String
    public let kind: String?

    public var description: String {
        if let kind = kind {
            return "fastbrowser[\(kind)]: \(message)"
        }
        return "fastbrowser: \(message)"
    }
}

/// fastbrowser 内核的 Swift 句柄（C FFI 薄封装）。
public final class FastBrowser {
    public init(engine: String = "mock", config: [String: Any] = [:]) throws {
        var cfg = config
        cfg["engine"] = engine
        let json = try JSONSerialization.data(withJSONObject: cfg)
        _ = try Self.call { json.withUnsafeBytes { buf in
            let s = String(decoding: buf, as: UTF8.self)
            return fastbrowser_init(s)
        } }
    }

    public static func version() -> String {
        guard let p = fastbrowser_version() else { return "" }
        let s = String(cString: p)
        fastbrowser_free_string(p)
        return s
    }

    public func open(_ url: String) throws -> [String: Any] {
        try (Self.call { fastbrowser_open(url) } as? [String: Any])
            ?? [:]
    }

    public func navigate(_ url: String) throws -> [String: Any] {
        try (Self.call { fastbrowser_navigate(url) } as? [String: Any])
            ?? [:]
    }

    public func toolList() throws -> [Any] {
        try (Self.call { fastbrowser_tool_list() } as? [Any]) ?? []
    }

    public func toolCall(_ name: String, params: [String: Any] = [:]) throws -> [String: Any] {
        let data = try JSONSerialization.data(withJSONObject: params)
        let json = String(decoding: data, as: UTF8.self)
        return try (Self.call { fastbrowser_tool_call(name, json) } as? [String: Any])
            ?? [:]
    }

    public func snapshot() throws -> [String: Any] {
        try (Self.call { fastbrowser_snapshot() } as? [String: Any]) ?? [:]
    }

    public func screenshot() throws -> [String: Any] {
        try (Self.call { fastbrowser_screenshot() } as? [String: Any]) ?? [:]
    }

    public func info() throws -> [String: Any] {
        try (Self.call { fastbrowser_get_info() } as? [String: Any]) ?? [:]
    }

    public func status() throws -> [String: Any] {
        try (Self.call { fastbrowser_status() } as? [String: Any]) ?? [:]
    }

    public func setViewport(width: UInt32, height: UInt32) throws -> [String: Any] {
        try (Self.call { fastbrowser_set_viewport(width, height) } as? [String: Any])
            ?? [:]
    }

    @discardableResult
    public func shutdown() throws -> [String: Any] {
        try (Self.call { fastbrowser_shutdown() } as? [String: Any]) ?? [:]
    }

    /// 调用返回 JSON 字符串的 C 函数并解析。
    fileprivate static func call(_ fn: () -> UnsafeMutablePointer<CChar>?) throws -> Any {
        guard let ptr = fn() else {
            throw FastbrowserError(message: "fastbrowser returned NULL", kind: "null")
        }
        let text = String(cString: ptr)
        fastbrowser_free_string(ptr)
        guard let data = text.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data)
        else {
            throw FastbrowserError(message: "bad JSON: \(text.prefix(120))…", kind: "json")
        }
        if let dict = obj as? [String: Any], let err = dict["error"] as? [String: Any] {
            throw FastbrowserError(
                message: err["message"] as? String ?? "unknown",
                kind: err["kind"] as? String
            )
        }
        return obj
    }
}

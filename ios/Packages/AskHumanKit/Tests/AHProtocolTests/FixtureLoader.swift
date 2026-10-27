import Foundation

/// Locates `ios/Fixtures/wire-v1` relative to this source file: the fixtures are shared with the
/// Rust tests and live in exactly one place.
enum Fixtures {
    static let directory: URL = {
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { url.deleteLastPathComponent() }  // AHProtocolTests, Tests, AskHumanKit, Packages, ios
        return url.appendingPathComponent("Fixtures/wire-v1", isDirectory: true)
    }()

    static func json(_ name: String) throws -> [String: Any] {
        let data = try Data(contentsOf: directory.appendingPathComponent(name))
        return try JSONSerialization.jsonObject(with: data) as! [String: Any]
    }
}

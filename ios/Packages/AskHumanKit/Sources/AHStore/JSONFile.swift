import Foundation

/// A Codable value persisted as one JSON file, written atomically with owner-only permissions.
public final class JSONFile<Value: Codable & Sendable>: @unchecked Sendable {
    public let url: URL
    private let lock = NSLock()

    public init(url: URL) { self.url = url }

    public func load() -> Value? {
        lock.lock(); defer { lock.unlock() }
        guard let data = try? Data(contentsOf: url) else { return nil }
        return try? JSONDecoder().decode(Value.self, from: data)
    }

    public func save(_ value: Value) {
        lock.lock(); defer { lock.unlock() }
        guard let data = try? JSONEncoder().encode(value) else { return }
        let dir = url.deletingLastPathComponent()
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try? data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    }

    public func remove() {
        lock.lock(); defer { lock.unlock() }
        try? FileManager.default.removeItem(at: url)
    }
}

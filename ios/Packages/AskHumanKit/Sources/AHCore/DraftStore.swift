import Foundation
import AHProtocol
import AHStore

/// Where half-written answers live between visits to a request.
public protocol DraftStore: AnyObject, Sendable {
    func draft(key: String) -> Draft?
    func save(_ draft: Draft, key: String)
    func remove(key: String)
    /// Directory holding the files attached to a draft.
    func filesDirectory(key: String) -> URL
}

extension LocalStore: DraftStore {
    public func save(_ draft: Draft, key: String) { saveDraft(draft, key: key) }
    public func remove(key: String) { removeDraft(key: key) }
    public func filesDirectory(key: String) -> URL { draftFiles(key: key) }
}

/// Drafts kept in memory (previews, tests, the mock backend).
public final class MemoryDraftStore: DraftStore, @unchecked Sendable {
    private let lock = NSLock()
    private var drafts: [String: Draft] = [:]
    private let root = FileManager.default.temporaryDirectory
        .appendingPathComponent("ah-drafts-\(UUID().uuidString.prefix(8))", isDirectory: true)

    public init() {}

    deinit { try? FileManager.default.removeItem(at: root) }

    public func draft(key: String) -> Draft? {
        lock.lock(); defer { lock.unlock() }
        return drafts[key]
    }

    public func save(_ draft: Draft, key: String) {
        lock.lock(); defer { lock.unlock() }
        if draft.isEmpty { drafts[key] = nil } else { drafts[key] = draft }
    }

    public func remove(key: String) {
        lock.lock(); defer { lock.unlock() }
        drafts[key] = nil
        try? FileManager.default.removeItem(at: filesDirectory(key: key))
    }

    public func filesDirectory(key: String) -> URL {
        root.appendingPathComponent(Bytes.b64u(Data(key.utf8)), isDirectory: true)
    }
}

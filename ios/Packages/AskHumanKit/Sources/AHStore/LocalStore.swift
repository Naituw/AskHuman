import Foundation
import AHProtocol

/// A directory of JSON files, one per key.
public final class JSONDirectory<Value: Codable & Sendable>: @unchecked Sendable {
    public let url: URL
    private let lock = NSLock()

    public init(url: URL) { self.url = url }

    private func file(_ key: String) -> URL {
        url.appendingPathComponent(Bytes.b64u(Data(key.utf8)) + ".json")
    }

    public func load(_ key: String) -> Value? {
        lock.lock(); defer { lock.unlock() }
        guard let data = try? Data(contentsOf: file(key)) else { return nil }
        return try? JSONDecoder().decode(Value.self, from: data)
    }

    public func all() -> [Value] {
        lock.lock(); defer { lock.unlock() }
        let files = (try? FileManager.default.contentsOfDirectory(at: url, includingPropertiesForKeys: nil)) ?? []
        return files.filter { $0.pathExtension == "json" }.compactMap {
            (try? Data(contentsOf: $0)).flatMap { try? JSONDecoder().decode(Value.self, from: $0) }
        }
    }

    public func save(_ value: Value, key: String) {
        lock.lock(); defer { lock.unlock() }
        guard let data = try? JSONEncoder().encode(value) else { return }
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        try? data.write(to: file(key), options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    }

    public func remove(_ key: String) {
        lock.lock(); defer { lock.unlock() }
        try? FileManager.default.removeItem(at: file(key))
    }

    public func removeAll() {
        lock.lock(); defer { lock.unlock() }
        try? FileManager.default.removeItem(at: url)
    }
}

/// App data on disk (Application Support): the request cache, drafts, local history, the outbox
/// and small per-Mac notes. All of it is replaceable; losing it never loses a pairing.
public final class LocalStore: @unchecked Sendable {
    public static let historyLimit = 500
    public static let historyMaxAge: TimeInterval = 90 * 24 * 3600

    public let root: URL
    private let requests: JSONDirectory<CachedRequest>
    private let drafts: JSONDirectory<Draft>
    private let outbox: JSONDirectory<OutboxItem>
    private let metas: JSONDirectory<MacMeta>
    private let historyFile: JSONFile<[HistoryEntry]>
    private let trafficFile: JSONFile<TrafficStats>
    private let lock = NSLock()

    public init(root: URL) {
        self.root = root
        requests = JSONDirectory(url: root.appendingPathComponent("requests", isDirectory: true))
        drafts = JSONDirectory(url: root.appendingPathComponent("drafts", isDirectory: true))
        outbox = JSONDirectory(url: root.appendingPathComponent("outbox", isDirectory: true))
        metas = JSONDirectory(url: root.appendingPathComponent("macs", isDirectory: true))
        historyFile = JSONFile(url: root.appendingPathComponent("history.json"))
        trafficFile = JSONFile(url: root.appendingPathComponent("traffic.json"))
    }

    // MARK: request cache

    public func cachedRequests() -> [CachedRequest] { requests.all() }
    public func saveRequest(_ item: CachedRequest) { requests.save(item, key: item.id) }
    public func removeRequest(macId: String, requestId: String) { requests.remove(CachedRequest.key(macId, requestId)) }

    // MARK: drafts

    public func draft(key: String) -> Draft? { drafts.load(key) }
    public func saveDraft(_ draft: Draft, key: String) {
        if draft.isEmpty { removeDraft(key: key) } else { drafts.save(draft, key: key) }
    }
    public func removeDraft(key: String) {
        drafts.remove(key)
        try? FileManager.default.removeItem(at: draftFiles(key: key))
    }

    /// Directory for the attachments of a draft.
    public func draftFiles(key: String) -> URL {
        root.appendingPathComponent("draft-files", isDirectory: true)
            .appendingPathComponent(Bytes.b64u(Data(key.utf8)), isDirectory: true)
    }

    // MARK: outbox

    public func outboxItems() -> [OutboxItem] {
        outbox.all().sorted { $0.createdAt < $1.createdAt }
    }
    public func saveOutbox(_ item: OutboxItem) { outbox.save(item, key: item.clientMsgId) }
    public func removeOutbox(_ clientMsgId: String) { outbox.remove(clientMsgId) }

    // MARK: per-Mac notes

    public func meta(macId: String) -> MacMeta { metas.load(macId) ?? MacMeta() }
    public func saveMeta(_ meta: MacMeta, macId: String) { metas.save(meta, key: macId) }

    /// Forgets everything stored for a Mac (it was unpaired).
    public func forget(macId: String) {
        for item in requests.all() where item.macId == macId { removeRequest(macId: item.macId, requestId: item.requestId) }
        for item in outbox.all() where item.macId == macId { removeOutbox(item.clientMsgId) }
        metas.remove(macId)
        lock.lock()
        let kept = (historyFile.load() ?? []).filter { $0.macId != macId }
        lock.unlock()
        saveHistory(kept)
    }

    // MARK: history

    public func history() -> [HistoryEntry] {
        lock.lock(); defer { lock.unlock() }
        return (historyFile.load() ?? []).sorted { $0.finishedAt > $1.finishedAt }
    }

    public func appendHistory(_ entry: HistoryEntry, now: Date = Date()) {
        lock.lock()
        var all = (historyFile.load() ?? []).filter { $0.id != entry.id }
        all.append(entry)
        lock.unlock()
        saveHistory(Self.prune(all, now: now))
    }

    public func removeHistory(id: String) {
        lock.lock()
        let kept = (historyFile.load() ?? []).filter { $0.id != id }
        lock.unlock()
        saveHistory(kept)
    }

    public func clearHistory() { saveHistory([]) }

    private func saveHistory(_ entries: [HistoryEntry]) {
        lock.lock(); defer { lock.unlock() }
        historyFile.save(entries)
    }

    static func prune(_ entries: [HistoryEntry], now: Date) -> [HistoryEntry] {
        let fresh = entries.filter { now.timeIntervalSince($0.finishedAt) <= historyMaxAge }
        let sorted = fresh.sorted { $0.finishedAt > $1.finishedAt }
        return Array(sorted.prefix(historyLimit))
    }

    // MARK: traffic

    public func traffic(month: String) -> TrafficStats {
        lock.lock(); defer { lock.unlock() }
        if let stats = trafficFile.load(), stats.month == month { return stats }
        return TrafficStats(month: month)
    }

    public func saveTraffic(_ stats: TrafficStats) {
        lock.lock(); defer { lock.unlock() }
        trafficFile.save(stats)
    }
}

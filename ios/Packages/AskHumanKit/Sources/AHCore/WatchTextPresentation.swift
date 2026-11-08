import Foundation
import AHProtocol

/// A completed paragraph from the Mac, independent of the tool/status refresh rate.
public struct WatchLatestText: Sendable, Equatable {
    public var id: String
    public var revision: String
    public var kind: String
    public var text: String
    public var truncated: Bool

    public init(id: String, revision: String, kind: String, text: String, truncated: Bool = false) {
        self.id = id
        self.revision = revision
        self.kind = kind
        self.text = text
        self.truncated = truncated
    }

    public init?(json: JSONValue) {
        guard let text = json["text"]?.stringValue, !text.isEmpty,
              let kind = json["kind"]?.stringValue, ["assistant", "thinking"].contains(kind),
              let revision = json["revision"]?.stringValue else { return nil }
        self.init(id: json["id"]?.stringValue ?? revision, revision: revision, kind: kind, text: text,
                  truncated: json["truncated"]?.boolValue ?? false)
    }
}

/// The view owns one instance so reading locks never affect other phones or the Mac.
public struct WatchTextPresentation: Sendable, Equatable {
    public private(set) var displayed: WatchLatestText?
    public private(set) var pending: WatchLatestText?
    public private(set) var expanded = false
    public private(set) var reading = false
    private var lastShown: Date?
    public static let interval: TimeInterval = 10

    public init() {}

    public var hasNewContent: Bool { pending != nil }
    public var deadline: Date? {
        guard pending != nil, !reading, !expanded, let lastShown else { return nil }
        return lastShown.addingTimeInterval(Self.interval)
    }

    public mutating func receive(_ candidate: WatchLatestText?, now: Date) {
        guard let candidate, !candidate.text.isEmpty else { return }
        if displayed?.revision == candidate.revision, displayed?.kind == candidate.kind {
            pending = nil
            return
        }
        pending = candidate
        if displayed == nil { showLatest(now: now) }
        else { tick(now: now) }
    }

    public mutating func tick(now: Date) {
        guard let deadline, now >= deadline else { return }
        showLatest(now: now)
    }

    public mutating func lockReading() { reading = true }

    public mutating func toggleExpanded(now: Date) {
        expanded.toggle()
        if expanded { reading = true }
        else {
            reading = false
            showLatest(now: now)
        }
    }

    public mutating func updateNow(now: Date) {
        showLatest(now: now)
        if !expanded { reading = false }
    }

    private mutating func showLatest(now: Date) {
        guard let pending else { return }
        displayed = pending
        self.pending = nil
        lastShown = now
    }
}

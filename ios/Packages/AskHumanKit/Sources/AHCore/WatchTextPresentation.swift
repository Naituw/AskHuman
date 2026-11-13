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

/// The view owns one instance so reading holds never affect other phones or the Mac.
///
/// New text replaces the shown text at most every `interval` seconds, unless somebody is reading it. Reading is a lease
/// (docs/specs/agent-observation-latest-text.md §4): a finger on the text holds it, and so does `lease` seconds after the
/// finger left, which a finger that comes back renews. An opened text holds until it is closed or the newest is asked for.
public struct WatchTextPresentation: Sendable, Equatable {
    public private(set) var displayed: WatchLatestText?
    public private(set) var pending: WatchLatestText?
    public private(set) var expanded = false
    /// A finger is on the text.
    public private(set) var touching = false
    /// After the last finger left the text, it holds still until this moment.
    private var heldUntil: Date?
    private var lastShown: Date?
    /// The shown text is replaced at most this often.
    public static let interval: TimeInterval = 10
    /// How long the text holds still after the finger that was on it has left.
    public static let lease: TimeInterval = 3

    public init() {}

    /// Whether the text is held, so that what is pending waits. A lease that has run out only stops being counted at the
    /// next `tick` or `receive`, which is also when anything waits for it.
    public var reading: Bool { expanded || touching || heldUntil != nil }
    public var hasNewContent: Bool { pending != nil }

    /// When the pending text is shown, if nothing holds it by then: the replacement interval after the last one, and not
    /// before the lease has run out. `nil` while the text is held without an end, or when nothing is pending.
    public var deadline: Date? {
        guard pending != nil, !expanded, !touching, let lastShown else { return nil }
        let due = lastShown.addingTimeInterval(Self.interval)
        guard let heldUntil else { return due }
        return max(due, heldUntil)
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
        endLeaseIfOver(now: now)
        guard let deadline, now >= deadline else { return }
        showLatest(now: now)
    }

    /// A finger came down on the text (and stayed, or moved over it).
    public mutating func touchBegan() {
        touching = true
        heldUntil = nil
    }

    /// The finger left the text.
    public mutating func touchEnded(now: Date) {
        guard touching else { return }
        touching = false
        heldUntil = now.addingTimeInterval(Self.lease)
    }

    public mutating func toggleExpanded(now: Date) {
        expanded.toggle()
        heldUntil = nil
        if !expanded { showLatest(now: now) }
    }

    public mutating func updateNow(now: Date) {
        showLatest(now: now)
        heldUntil = nil
    }

    private mutating func endLeaseIfOver(now: Date) {
        if let heldUntil, now >= heldUntil { self.heldUntil = nil }
    }

    private mutating func showLatest(now: Date) {
        guard let pending else { return }
        displayed = pending
        self.pending = nil
        lastShown = now
    }
}

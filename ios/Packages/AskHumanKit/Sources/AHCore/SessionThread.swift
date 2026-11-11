import Foundation
import Observation
import AHProtocol

// The earlier exchanges of the Agent session a request belongs to (docs/specs/ios-session-history.md §7.2), loaded a
// page at a time while the person reads the request page. The page owns it: leaving the page drops it (S14), nothing
// is written to disk, and the Mac stays the only place the history is kept.

@MainActor
@Observable
public final class SessionThread {
    public enum Phase: Equatable, Sendable {
        /// Nothing was asked for yet.
        case idle
        case loading
        /// The last page could not be had; what was loaded stays, and asking again tries once more.
        case failed(BackendError)
        case ready
    }

    /// Fetches the exchanges older than a cursor: `(beforeMs, beforeId, limit)`.
    public typealias Loader = @Sendable (_ beforeMs: Int64, _ beforeId: String?, _ limit: Int) async throws -> SessionHistory

    /// Oldest first, the order the page shows them in: a page that arrives is put in front of what is there.
    public private(set) var items: [SessionExchange] = []
    public private(set) var phase: Phase = .idle
    /// The Mac said there is nothing older than what is shown (or, with nothing shown, than the request).
    public private(set) var reachedStart = false
    /// Counts the times the person asked for the history (a pull, the menu, a retry). The page gives up a reading
    /// position it still waits to restore when this changes: the person has just chosen where to look.
    public private(set) var intent = 0

    @ObservationIgnored private let anchorMs: Int64
    @ObservationIgnored private let pageSize: Int
    @ObservationIgnored private let load: Loader
    @ObservationIgnored private var loadedIds: Set<String> = []

    /// `anchorMs` is when the request being read was asked: only what was answered before it is its history.
    public init(anchorMs: Int64, pageSize: Int = Wire.sessionHistoryPage, load: @escaping Loader) {
        self.anchorMs = anchorMs
        self.pageSize = pageSize
        self.load = load
    }

    /// A first page was asked for at some point (whatever came of it).
    public var wasOpened: Bool { phase != .idle }

    /// The person asked for the history. `loadMore` calls it; the menu calls it when everything is loaded already.
    public func asked() { intent += 1 }

    /// Loads the next older page (the first one while nothing is loaded). One request at a time: a call while one is
    /// in flight, or after the start was reached, does nothing. `beforeInsert` runs once the page is in hand and
    /// before it is shown, so the page can prepare for the content that is about to appear above what it shows.
    /// Returns whether a page was added.
    @discardableResult
    public func loadMore(beforeInsert: @MainActor () async -> Void = {}) async -> Bool {
        guard phase != .loading, !reachedStart else { return false }
        asked()
        phase = .loading
        let cursor = items.first.map { (ms: $0.at, id: Optional($0.id)) } ?? (ms: anchorMs, id: nil)
        let page: SessionHistory
        do {
            page = try await load(cursor.ms, cursor.id, pageSize)
        } catch {
            if error is CancellationError || (error as? BackendError) == .cancelled {
                // Nobody is waiting for the answer any more; the next ask starts from where this one did.
                phase = items.isEmpty && !reachedStart ? .idle : .ready
            } else {
                phase = .failed((error as? BackendError) ?? .unavailable("failed"))
                Diagnostics.note("history", "session page failed (\(items.count) loaded)")
            }
            return false
        }
        await beforeInsert()
        // A page is newest first; the thread is oldest first. An exchange that is here already is not shown twice.
        let fresh = page.items.reversed().filter { loadedIds.insert($0.id).inserted }
        items.insert(contentsOf: fresh, at: 0)
        reachedStart = !page.more
        phase = .ready
        Diagnostics.note("history", "session page: \(fresh.count) added, \(items.count) loaded, more=\(page.more)")
        return !fresh.isEmpty
    }
}

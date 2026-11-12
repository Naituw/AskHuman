import Foundation
import AHProtocol
import AHStore

extension AppStore {
    /// Whether the request page can show the earlier exchanges of its session (docs/specs/ios-session-history.md
    /// §7.1): the request names a real Agent session, the Mac can read it back, and the full request is here (the
    /// inbox summary has no session key, so a page built from a summary alone has no history yet).
    public func sessionHistoryAvailable(for item: InboxItem) -> Bool {
        guard let key = item.view?.agentKey, !key.isEmpty else { return false }
        return state.macs.first { $0.id == item.macId }?.caps.sessionHistory == true
    }

    /// A thread for the request page to hold, `nil` when this request has no history to show. Not cached here:
    /// the page owns the thread, and leaving the page drops it.
    public func makeSessionThread(for item: InboxItem) -> SessionThread? {
        guard sessionHistoryAvailable(for: item), let key = item.view?.agentKey else { return nil }
        let backend = backend
        let macId = item.macId
        return SessionThread(anchorMs: item.summary.createdAt) { beforeMs, beforeId, limit in
            try await backend.sessionHistory(macId: macId, agentKey: key, beforeMs: beforeMs, beforeId: beforeId, limit: limit)
        }
    }

    /// What the Mac still has of a finished request's exchange, pictures included (docs/specs/ios-history-pictures.md
    /// HP9), for the detail page of the History tab. `nil` is every way of not getting it: the Mac is offline, is older,
    /// has forgotten the request, or does not answer in time. The page is then what it was before the pictures, so none
    /// of them is an error to show.
    public func historyExchange(for entry: HistoryEntry) async -> SessionExchange? {
        guard state.macs.first(where: { $0.id == entry.macId })?.caps.historyPictures == true else { return nil }
        return try? await backend.sessionExchange(macId: entry.macId, id: entry.request.id)
    }
}

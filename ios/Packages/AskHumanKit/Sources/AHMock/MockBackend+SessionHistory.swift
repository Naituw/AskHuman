import Foundation
import AHProtocol
import AHCore

// Session history of the mock Macs (docs/specs/ios-session-history.md): pages of earlier exchanges, cut by the cursor
// the real Mac uses, so the screens and the thread model meet the same paging they will on a real Mac.

extension MockBackend {
    private func requireSessionHistoryMac(_ macId: String) throws {
        guard let mac = state.macs.first(where: { $0.id == macId }) else { throw BackendError.unknownMac }
        if unreachable.contains(macId) || mac.link != .online { throw BackendError.macOffline }
        guard mac.caps.sessionHistory else { throw BackendError.rejected("unsupported") }
    }

    public func sessionHistory(
        macId: String, agentKey: String, beforeMs: Int64, beforeId: String?, limit: Int
    ) async throws -> SessionHistory {
        try requireSessionHistoryMac(macId)
        sessionHistoryCalls.append(
            SessionHistoryCall(macId: macId, agentKey: agentKey, beforeMs: beforeMs, beforeId: beforeId, limit: limit)
        )
        await waitForSessionHistoryRead()
        try requireSessionHistoryMac(macId)
        if failNextSessionHistory > 0 {
            failNextSessionHistory -= 1
            throw BackendError.timedOut
        }
        let limit = min(max(limit, 1), Wire.sessionHistoryPageMax)
        // Newest first already; older means below the cursor `(beforeMs, beforeId)`, or `at < beforeMs` without an id.
        let older = (sessionHistories[agentKey] ?? []).filter { exchange in
            guard let beforeId else { return exchange.at < beforeMs }
            return exchange.at < beforeMs || (exchange.at == beforeMs && exchange.id < beforeId)
        }
        return SessionHistory(reqId: "mock", agentKey: agentKey, items: Array(older.prefix(limit)), more: older.count > limit)
    }

    /// A read takes its delay, then waits for the gate a UI test may have set.
    private func waitForSessionHistoryRead() async {
        await pause(sessionHistoryDelay)
        if let gate = sessionHistoryGate {
            // A test that has to do something while the load is in flight decides when the answer is ready, instead of
            // racing a timer against a test runner that can be seconds late on a busy machine.
            let giveUp = Date().addingTimeInterval(60)
            while !FileManager.default.fileExists(atPath: gate), Date() < giveUp {
                try? await Task.sleep(nanoseconds: 50_000_000)
            }
        }
    }

    /// One exchange of the Mac's history by its id, with its pictures (docs/specs/ios-history-pictures.md HP9); `nil`
    /// when the Mac holds no such record. Waits like a read of a session does, so a test can tell the page from its
    /// pictures.
    public func sessionExchange(macId: String, id: String) async throws -> SessionExchange? {
        guard let mac = state.macs.first(where: { $0.id == macId }) else { throw BackendError.unknownMac }
        if unreachable.contains(macId) || mac.link != .online { throw BackendError.macOffline }
        guard mac.caps.historyPictures else { throw BackendError.rejected("unsupported") }
        sessionExchangeCalls.append(SessionExchangeCall(macId: macId, id: id))
        await waitForSessionHistoryRead()
        return sessionHistories.values.lazy.flatMap { $0 }.first { $0.id == id }
    }

    /// What the Mac holds of a session (a test sets up its own), kept newest first the way the Mac orders it.
    public func setSessionHistory(_ items: [SessionExchange], for agentKey: String) {
        sessionHistories[agentKey] = items.sorted { ($0.at, $0.id) > ($1.at, $1.id) }
    }

    public func setFailNextSessionHistory(_ count: Int) {
        failNextSessionHistory = count
    }

    /// Makes a read take this many real seconds, whatever the latency is set to (the pull's UI tests need a load that
    /// lasts long enough to scroll during, and one that is over before the page has settled).
    public func setSessionHistoryDelay(realSeconds: Double) {
        sessionHistoryDelay = latency > 0 ? max(0, realSeconds) / latency : 0
    }

    /// Holds every read back until the file at `path` exists (the delay counts first).
    public func setSessionHistoryGate(path: String) {
        sessionHistoryGate = path
    }
}

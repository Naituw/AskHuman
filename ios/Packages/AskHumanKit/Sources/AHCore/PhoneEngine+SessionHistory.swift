import Foundation
import AHProtocol

// A session's earlier exchanges (docs/specs/ios-session-history.md §7.2): one read-only request with one answer.
// It travels through the same mailboxes as the todo reads (`PhoneEngine+Todos.swift`), so a dropped link, an
// offline Mac and a removed pairing fail it without a mechanism of its own. Nothing is kept here or on disk:
// the history lives on the Mac, the phone only shows what the Mac last said.

extension PhoneEngine {
    static func sessionHistoryKey(_ macId: String, _ reqId: String) -> String { "h|" + macId + "|" + reqId }
    static func historyExchangeKey(_ macId: String, _ reqId: String) -> String { "x|" + macId + "|" + reqId }

    func onSessionHistory(_ macId: String, _ page: SessionHistory) {
        deliverTodo(.history(page), key: Self.sessionHistoryKey(macId, page.reqId), from: macId)
    }

    func onHistoryExchange(_ macId: String, _ answer: HistoryExchange) {
        deliverTodo(.exchange(answer), key: Self.historyExchangeKey(macId, answer.reqId), from: macId)
    }

    public func sessionHistory(
        macId: String, agentKey: String, beforeMs: Int64, beforeId: String?, limit: Int
    ) async throws -> SessionHistory {
        sessionHistorySeq += 1
        let reqId = String(sessionHistorySeq)
        // A Mac that describes the pictures of the exchanges is asked to (docs/specs/ios-history-pictures.md §7);
        // an older one is asked what it always was.
        let pictures = macs[macId]?.caps?.historyPictures == true
        let reply = try await todoRequest(
            SessionHistoryGet(
                reqId: reqId, agentKey: agentKey, beforeMs: beforeMs, beforeId: beforeId, limit: limit, pictures: pictures
            ),
            macId: macId, key: Self.sessionHistoryKey(macId, reqId), timeout: Self.todoReadTimeout,
            supported: { $0.sessionHistory }
        )
        guard case .history(let page) = reply, page.agentKey == agentKey else { throw BackendError.invalid("reply") }
        return page
    }

    /// One exchange of the Mac's history by its id (the id of the request it answered), pictures included; `nil` when
    /// the Mac has no such record. Throws like `sessionHistory`, and `.rejected("unsupported")` for a Mac that does
    /// not describe pictures.
    public func sessionExchange(macId: String, id: String) async throws -> SessionExchange? {
        sessionHistorySeq += 1
        let reqId = String(sessionHistorySeq)
        let reply = try await todoRequest(
            HistoryExchangeGet(reqId: reqId, id: id),
            macId: macId, key: Self.historyExchangeKey(macId, reqId), timeout: Self.todoReadTimeout,
            supported: { $0.historyPictures }
        )
        guard case .exchange(let answer) = reply, answer.id == id else { throw BackendError.invalid("reply") }
        return answer.item
    }
}

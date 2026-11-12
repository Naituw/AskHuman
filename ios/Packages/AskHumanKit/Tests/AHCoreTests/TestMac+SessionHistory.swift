import Foundation
import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// The session history side of the fake Mac: pages of earlier exchanges, cut by the cursor the real Mac uses.
extension TestMac {
    func handleSessionHistory(_ plain: Plain, from phoneId: String) async {
        guard let get = try? plain.body(as: SessionHistoryGet.self) else { return }
        let (ignore, answer, held) = lock.withLock {
            sessionHistoryGets.append(get)
            return (ignoreSessionHistory, sessionHistoryAnswer, sessionHistories[get.agentKey] ?? [])
        }
        guard !ignore else { return }
        let page: SessionHistory?
        if let answer { page = answer(get) } else { page = Self.page(for: get, in: held) }
        if let page { await sendDown(page, to: phoneId) }
    }

    /// One exchange by its id: the answer says whether the Mac has it.
    func handleHistoryExchange(_ plain: Plain, from phoneId: String) async {
        guard let get = try? plain.body(as: HistoryExchangeGet.self) else { return }
        let (ignore, answer, held) = lock.withLock {
            historyExchangeGets.append(get)
            return (ignoreHistoryExchange, historyExchangeAnswer, historyExchanges[get.id])
        }
        guard !ignore else { return }
        let reply: HistoryExchange?
        if let answer { reply = answer(get) } else { reply = HistoryExchange(reqId: get.reqId, id: get.id, item: held) }
        if let reply { await sendDown(reply, to: phoneId) }
    }

    /// The page the Mac sends for `get` out of `held` (newest first): what is older than the cursor `(beforeMs, beforeId)`,
    /// or than `beforeMs` alone without an id, at most `limit` of it, and whether any is left.
    static func page(for get: SessionHistoryGet, in held: [SessionExchange]) -> SessionHistory {
        let older = held.filter { exchange in
            guard let beforeId = get.beforeId else { return exchange.at < get.beforeMs }
            return exchange.at < get.beforeMs || (exchange.at == get.beforeMs && exchange.id < beforeId)
        }
        let limit = min(max(get.limit, 1), Wire.sessionHistoryPageMax)
        return SessionHistory(reqId: get.reqId, agentKey: get.agentKey, items: Array(older.prefix(limit)), more: older.count > limit)
    }

    /// `count` exchanges of a session, newest first, `at` falling by one second each from `newest`.
    static func exchanges(_ count: Int, newest: Int64 = 1_800_000_000_000) -> [SessionExchange] {
        (0..<count).map { index in
            SessionExchange(
                id: "e\(count - index)", at: newest - Int64(index) * 1_000, channel: "popup", message: "message \(count - index)",
                qs: [SessionQuestion(text: "question \(count - index)", selected: ["yes"])]
            )
        }
    }
}

import XCTest
import AHProtocol
import AHStore
import AHMock
@testable import AHCore

/// The request page's side of the session history (docs/specs/ios-session-history.md §7.1/§7.2) against the mock
/// Macs: when a request has a history at all, and what the thread it gets shows.
@MainActor
final class SessionHistoryStoreTests: XCTestCase {
    private var backend: MockBackend!
    private var app: AppStore!

    private func make() async {
        backend = MockBackend(scenario: .sessionHistory, latency: 0)
        let defaults = UserDefaults(suiteName: "ah-sessionhistory-test-" + UUID().uuidString)!
        app = AppStore(backend: backend, drafts: MemoryDraftStore(), defaults: defaults)
        await app.start()
        await eventually("the Macs and the inbox") {
            self.app.state.foreground && self.app.state.macs.count == 2 && self.app.state.inbox.count == 7
        }
    }

    private func item(_ requestId: String) throws -> InboxItem {
        try XCTUnwrap(app.state.inbox.first { $0.requestId == requestId })
    }

    private func thread(_ requestId: String) throws -> SessionThread {
        try XCTUnwrap(app.makeSessionThread(for: try item(requestId)), "\(requestId) has a history")
    }

    // MARK: when a request has a history

    func testOnlyARequestOfARealSessionOnAMacThatReadsItBackHasAHistory() async throws {
        await make()
        XCTAssertTrue(app.sessionHistoryAvailable(for: try item("req-history-long")))
        XCTAssertTrue(app.sessionHistoryAvailable(for: try item("req-history-confirm")), "a permission request too")
        XCTAssertTrue(app.sessionHistoryAvailable(for: try item("req-history-empty")), "an empty session still answers: nothing earlier")
        XCTAssertFalse(app.sessionHistoryAvailable(for: try item("req-history-none")), "no session behind the request")
        XCTAssertFalse(app.sessionHistoryAvailable(for: try item("req-history-old-mac")), "the Mac cannot read it back")
        XCTAssertNil(app.makeSessionThread(for: try item("req-history-none")))
        XCTAssertNil(app.makeSessionThread(for: try item("req-history-old-mac")))
    }

    func testARequestKnownOnlyBySummaryHasNoHistoryYet() async throws {
        await make()
        var summaryOnly = try item("req-history-long")
        summaryOnly.view = nil
        XCTAssertFalse(app.sessionHistoryAvailable(for: summaryOnly), "the summary carries no session key")
        XCTAssertNil(app.makeSessionThread(for: summaryOnly))
    }

    func testTheDemoScenarioHasNoHistoryAnywhere() async throws {
        backend = MockBackend(scenario: .demo, latency: 0)
        app = AppStore(backend: backend, drafts: MemoryDraftStore(), defaults: UserDefaults(suiteName: "ah-sessionhistory-demo-" + UUID().uuidString)!)
        await app.start()
        await eventually("the inbox") { !self.app.state.inbox.isEmpty }
        for item in app.state.inbox {
            XCTAssertFalse(app.sessionHistoryAvailable(for: item), "\(item.requestId) has no entry")
        }
    }

    // MARK: what the thread shows

    func testPagesComeNewestFirstAndAreShownOldestFirstUpToTheStart() async throws {
        await make()
        let thread = try thread("req-history-long")
        await thread.loadMore()
        XCTAssertEqual(thread.items.count, 8)
        XCTAssertEqual(thread.items.last?.id, "ex-long-23", "the newest exchange is next to the request")
        XCTAssertEqual(thread.items.first?.id, "ex-long-16")

        await thread.loadMore()
        XCTAssertEqual(thread.items.count, 16)
        XCTAssertFalse(thread.reachedStart)
        await thread.loadMore()
        XCTAssertEqual(thread.items.count, MockData.longSessionCount)
        XCTAssertTrue(thread.reachedStart)
        XCTAssertEqual(thread.items.first?.id, "ex-long-01")
        XCTAssertEqual(thread.items.map(\.id), thread.items.map(\.id).sorted(), "oldest first, without a gap")

        let calls = await backend.sessionHistoryCalls
        XCTAssertEqual(calls.count, 3)
        XCTAssertTrue(calls.allSatisfy { $0.agentKey == MockData.longSessionKey && $0.macId == MockData.homeMacId && $0.limit == 8 })
        XCTAssertEqual(calls[0].beforeMs, try item("req-history-long").summary.createdAt, "the first page starts at the request")
        XCTAssertNil(calls[0].beforeId)
        XCTAssertEqual(calls[1].beforeId, "ex-long-16", "the next page starts where the shown ones begin")
    }

    func testAShortSessionIsOnePageThatReachesTheStart() async throws {
        await make()
        let thread = try thread("req-history-short")
        await thread.loadMore()
        XCTAssertEqual(thread.items.map(\.id), ["ex-short-01", "ex-short-02", "ex-short-03"])
        XCTAssertTrue(thread.reachedStart)
    }

    func testASessionWithNothingEarlierIsReadyAtItsStart() async throws {
        await make()
        let thread = try thread("req-history-empty")
        await thread.loadMore()
        XCTAssertTrue(thread.items.isEmpty)
        XCTAssertEqual(thread.phase, .ready)
        XCTAssertTrue(thread.reachedStart)
    }

    func testTheRequestsOwnExchangeIsNotItsHistory() async throws {
        await make()
        // An exchange answered after the request was asked (or at that very moment) belongs to the request, not to what
        // came before it.
        let now = try item("req-history-short").summary.createdAt
        let held = (1...5).map { number in
            SessionExchange(id: "mine-\(number)", at: now + Int64(number - 3) * 1_000, channel: "popup", message: "m", qs: [])
        }
        await backend.setSessionHistory(held, for: MockData.shortSessionKey)
        let thread = try thread("req-history-short")
        await thread.loadMore()
        XCTAssertEqual(thread.items.map(\.id), ["mine-1", "mine-2"], "only what is older than the request")
    }

    // MARK: failing

    func testAReadThatTimesOutCanBeRetriedAndKeepsWhatWasLoaded() async throws {
        await make()
        let thread = try thread("req-history-long")
        await thread.loadMore()
        await backend.setFailNextSessionHistory(1)
        await thread.loadMore()
        XCTAssertEqual(thread.phase, .failed(.timedOut))
        XCTAssertEqual(thread.items.count, 8)

        await thread.loadMore()
        XCTAssertEqual(thread.phase, .ready)
        XCTAssertEqual(thread.items.count, 16)
    }

    func testAMacThatWentAwayIsOfflineForTheThread() async throws {
        await make()
        let thread = try thread("req-history-long")
        await backend.setUnreachable([MockData.homeMacId])
        await thread.loadMore()
        XCTAssertEqual(thread.phase, .failed(.macOffline))
        XCTAssertTrue(thread.items.isEmpty)
        XCTAssertTrue(thread.wasOpened)

        await backend.setUnreachable([])
        await thread.loadMore()
        XCTAssertEqual(thread.items.count, 8)
    }

    func testTheMockRefusesAMacThatCannotReadHistory() async throws {
        await make()
        do {
            _ = try await backend.sessionHistory(
                macId: MockData.officeMacId, agentKey: MockData.longSessionKey, beforeMs: Int64.max, beforeId: nil, limit: 8
            )
            XCTFail("unsupported")
        } catch BackendError.rejected(let reason) {
            XCTAssertEqual(reason, "unsupported")
        }
    }
}

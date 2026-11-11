import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// The engine's side of the session history (docs/specs/ios-session-history.md §7.2): the request and its answer, the
/// cursor, a Mac that cannot answer, timeouts, and a Mac that goes away.
final class SessionHistoryEngineTests: XCTestCase {
    private let key = "claude:session-1"
    private let capable = Caps(newTask: true, interject: true, watch: true, sessionHistory: true)

    private func pairedHarness(history: Bool = true, held: Int = 12) async -> Harness {
        let h = Harness()
        h.mac.caps = history ? capable : Caps(newTask: true, interject: true, watch: true)
        h.mac.sessionHistories[key] = TestMac.exchanges(held)
        await h.pair()
        return h
    }

    // MARK: pages

    func testAPageComesBackNewestFirstWithTheCursorItWasAskedWith() async throws {
        let h = await pairedHarness()
        let first = try await h.engine.sessionHistory(
            macId: h.mac.macId, agentKey: key, beforeMs: Int64.max, beforeId: nil, limit: 8
        )
        XCTAssertEqual(first.items.map(\.id), (5...12).reversed().map { "e\($0)" })
        XCTAssertTrue(first.more)
        XCTAssertEqual(first.agentKey, key)

        let oldest = try XCTUnwrap(first.items.last)
        let second = try await h.engine.sessionHistory(
            macId: h.mac.macId, agentKey: key, beforeMs: oldest.at, beforeId: oldest.id, limit: 8
        )
        XCTAssertEqual(second.items.map(\.id), ["e4", "e3", "e2", "e1"])
        XCTAssertFalse(second.more)

        let gets = h.mac.sessionHistoryGets
        XCTAssertEqual(gets.count, 2)
        XCTAssertEqual(gets[0].beforeMs, Int64.max)
        XCTAssertNil(gets[0].beforeId)
        XCTAssertEqual(gets[0].limit, 8)
        XCTAssertEqual(gets[1].beforeMs, oldest.at)
        XCTAssertEqual(gets[1].beforeId, oldest.id)
        XCTAssertNotEqual(gets[0].reqId, gets[1].reqId, "every read has a number of its own")
    }

    func testASessionTheMacHoldsNothingOfIsAnEmptyPageNotAnError() async throws {
        let h = await pairedHarness()
        let page = try await h.engine.sessionHistory(
            macId: h.mac.macId, agentKey: "codex:nobody", beforeMs: Int64.max, beforeId: nil, limit: 8
        )
        XCTAssertTrue(page.items.isEmpty)
        XCTAssertFalse(page.more)
    }

    func testTwoReadsAtOnceAreEachAnsweredWithTheirOwnPage() async throws {
        let h = await pairedHarness()
        let mac = h.mac.macId, key = key
        async let first = h.engine.sessionHistory(macId: mac, agentKey: key, beforeMs: Int64.max, beforeId: nil, limit: 2)
        async let second = h.engine.sessionHistory(macId: mac, agentKey: key, beforeMs: Int64.max, beforeId: nil, limit: 5)
        let (a, b) = try await (first, second)
        XCTAssertEqual(a.items.count, 2)
        XCTAssertEqual(b.items.count, 5)
    }

    func testAnAnswerNobodyWaitsForIsDroppedAndTheNextReadIsUnharmed() async throws {
        let h = await pairedHarness()
        await h.mac.sendDown(SessionHistory(reqId: "stray", agentKey: key, items: TestMac.exchanges(2), more: false), to: h.mac.pairedPhones[0])
        let page = try await h.engine.sessionHistory(macId: h.mac.macId, agentKey: key, beforeMs: Int64.max, beforeId: nil, limit: 3)
        XCTAssertEqual(page.items.count, 3)
    }

    func testAPageOfAnotherSessionIsNotTakenForTheAnswer() async throws {
        let h = await pairedHarness()
        h.mac.sessionHistoryAnswer = { get in
            SessionHistory(reqId: get.reqId, agentKey: "codex:someone-else", items: TestMac.exchanges(2), more: false)
        }
        do {
            _ = try await h.engine.sessionHistory(macId: h.mac.macId, agentKey: key, beforeMs: Int64.max, beforeId: nil, limit: 8)
            XCTFail("a page of another session")
        } catch BackendError.invalid {}
    }

    // MARK: what cannot be asked

    func testAMacWithoutTheCapabilityIsNotAsked() async throws {
        let h = await pairedHarness(history: false)
        do {
            _ = try await h.engine.sessionHistory(macId: h.mac.macId, agentKey: key, beforeMs: Int64.max, beforeId: nil, limit: 8)
            XCTFail("unsupported")
        } catch BackendError.rejected(let reason) {
            XCTAssertEqual(reason, "unsupported")
        }
        XCTAssertEqual(h.mac.count(SessionHistoryGet.type), 0, "nothing goes out to a Mac that cannot answer")
    }

    func testAnUnknownMacIsOffline() async throws {
        let h = await pairedHarness()
        do {
            _ = try await h.engine.sessionHistory(macId: "nobody", agentKey: key, beforeMs: Int64.max, beforeId: nil, limit: 8)
            XCTFail("offline")
        } catch BackendError.macOffline {}
    }

    // MARK: failing

    func testNoAnswerIsATimeoutAndTheNextReadIsServed() async throws {
        let h = await pairedHarness()
        h.mac.ignoreSessionHistory = true
        let macId = h.mac.macId, key = key
        let task = Task { try await h.engine.sessionHistory(macId: macId, agentKey: key, beforeMs: Int64.max, beforeId: nil, limit: 8) }
        await eventually("the Mac to get the request") { h.mac.count(SessionHistoryGet.type) == 1 }
        await h.pass(seconds: Int(PhoneEngine.todoReadTimeout) + 2)
        do {
            _ = try await task.value
            XCTFail("timeout")
        } catch BackendError.timedOut {}

        h.mac.ignoreSessionHistory = false
        let page = try await h.engine.sessionHistory(macId: macId, agentKey: key, beforeMs: Int64.max, beforeId: nil, limit: 8)
        XCTAssertEqual(page.items.count, 8)
    }

    func testAMacThatGoesAwayFailsTheReadAtOnce() async throws {
        let h = await pairedHarness()
        h.mac.ignoreSessionHistory = true
        let macId = h.mac.macId, key = key
        let task = Task { try await h.engine.sessionHistory(macId: macId, agentKey: key, beforeMs: Int64.max, beforeId: nil, limit: 8) }
        await eventually("the Mac to get the request") { h.mac.count(SessionHistoryGet.type) == 1 }
        await h.mac.publishPresence(online: false)
        do {
            _ = try await task.value
            XCTFail("offline")
        } catch BackendError.macOffline {}
    }

    func testACancelledReadStopsWaiting() async throws {
        let h = await pairedHarness()
        h.mac.ignoreSessionHistory = true
        let macId = h.mac.macId, key = key
        let task = Task { try await h.engine.sessionHistory(macId: macId, agentKey: key, beforeMs: Int64.max, beforeId: nil, limit: 8) }
        await eventually("the Mac to get the request") { h.mac.count(SessionHistoryGet.type) == 1 }
        task.cancel()
        do {
            _ = try await task.value
            XCTFail("cancelled")
        } catch BackendError.cancelled {}
    }
}

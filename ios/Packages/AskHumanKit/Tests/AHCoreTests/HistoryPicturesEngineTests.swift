import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// The engine's side of the pictures in the history (docs/specs/ios-history-pictures.md §7): a page is asked for pictures
/// only of a Mac that describes them, and one exchange can be read by its id, with the same mailbox, timeout and
/// offline rules as the pages of a session.
final class HistoryPicturesEngineTests: XCTestCase {
    private let key = "claude:session-1"
    private let described = Caps(newTask: true, interject: true, watch: true, sessionHistory: true, historyPictures: true)
    private let readsOnly = Caps(newTask: true, interject: true, watch: true, sessionHistory: true)

    private func pairedHarness(caps: Caps) async -> Harness {
        let h = Harness()
        h.mac.caps = caps
        h.mac.sessionHistories[key] = TestMac.exchanges(12)
        await h.pair()
        return h
    }

    private func picture(_ id: String, gone: Bool = false) -> AssetRef {
        AssetRef(
            assetId: id, name: id + ".png", size: gone ? 0 : 2_048, mime: "image/png", isImage: true, thumb: gone ? nil : "/9j/4AAQ",
            gone: gone
        )
    }

    private func exchange(_ id: String) -> SessionExchange {
        SessionExchange(
            id: id, at: 1_800_000_000_000, channel: "popup", message: "screens", attachments: 3,
            pictures: [picture("p1"), picture("p2", gone: true)],
            qs: [SessionQuestion(text: "like this?", input: "yes", images: 1, pictures: [picture("r1")])]
        )
    }

    // MARK: pages

    func testAPageIsAskedForPicturesOnlyOfAMacThatDescribesThem() async throws {
        let capable = await pairedHarness(caps: described)
        _ = try await capable.engine.sessionHistory(macId: capable.mac.macId, agentKey: key, beforeMs: Int64.max, beforeId: nil, limit: 8)
        XCTAssertEqual(capable.mac.sessionHistoryGets.map(\.pictures), [true])

        let older = await pairedHarness(caps: readsOnly)
        _ = try await older.engine.sessionHistory(macId: older.mac.macId, agentKey: key, beforeMs: Int64.max, beforeId: nil, limit: 8)
        XCTAssertEqual(older.mac.sessionHistoryGets.map(\.pictures), [false], "what an older Mac is asked is what it always was")
    }

    func testThePicturesOfAPageComeThroughAsTheMacSentThem() async throws {
        let h = await pairedHarness(caps: described)
        let held = exchange("e1")
        h.mac.sessionHistories[key] = [held]
        let page = try await h.engine.sessionHistory(macId: h.mac.macId, agentKey: key, beforeMs: Int64.max, beforeId: nil, limit: 8)
        XCTAssertEqual(page.items, [held])
        XCTAssertEqual(page.items.first?.pictures.map(\.gone), [false, true])
        XCTAssertEqual(page.items.first?.qs.first?.pictures.map(\.assetId), ["r1"])
    }

    // MARK: one exchange

    func testAnExchangeComesBackWithItsPictures() async throws {
        let h = await pairedHarness(caps: described)
        h.mac.historyExchanges["req-7"] = exchange("req-7")
        let item = try await h.engine.sessionExchange(macId: h.mac.macId, id: "req-7")
        XCTAssertEqual(item, exchange("req-7"))
        XCTAssertEqual(h.mac.historyExchangeGets.map(\.id), ["req-7"])
    }

    func testARecordTheMacDoesNotHaveIsNothingNotAnError() async throws {
        let h = await pairedHarness(caps: described)
        let item = try await h.engine.sessionExchange(macId: h.mac.macId, id: "req-gone")
        XCTAssertNil(item)
        XCTAssertEqual(h.mac.historyExchangeGets.count, 1, "the Mac was asked, and said it has none")
    }

    func testAMacThatDoesNotDescribePicturesIsNotAsked() async throws {
        let h = await pairedHarness(caps: readsOnly)
        do {
            _ = try await h.engine.sessionExchange(macId: h.mac.macId, id: "req-7")
            XCTFail("unsupported")
        } catch BackendError.rejected(let reason) {
            XCTAssertEqual(reason, "unsupported")
        }
        XCTAssertEqual(h.mac.count(HistoryExchangeGet.type), 0, "nothing goes out to a Mac that cannot answer")
    }

    func testAnUnknownMacIsOffline() async throws {
        let h = await pairedHarness(caps: described)
        do {
            _ = try await h.engine.sessionExchange(macId: "nobody", id: "req-7")
            XCTFail("offline")
        } catch BackendError.macOffline {}
    }

    func testTheExchangeOfAnotherRequestIsNotTakenForTheAnswer() async throws {
        let h = await pairedHarness(caps: described)
        h.mac.historyExchangeAnswer = { get in HistoryExchange(reqId: get.reqId, id: "someone-else", item: nil) }
        do {
            _ = try await h.engine.sessionExchange(macId: h.mac.macId, id: "req-7")
            XCTFail("an answer about another record")
        } catch BackendError.invalid {}
    }

    func testAnExchangeAndAPageAtOnceAreEachAnsweredWithTheirOwn() async throws {
        let h = await pairedHarness(caps: described)
        h.mac.historyExchanges["req-7"] = exchange("req-7")
        let mac = h.mac.macId, key = key
        async let page = h.engine.sessionHistory(macId: mac, agentKey: key, beforeMs: Int64.max, beforeId: nil, limit: 3)
        async let item = h.engine.sessionExchange(macId: mac, id: "req-7")
        let (got, one) = try await (page, item)
        XCTAssertEqual(got.items.count, 3)
        XCTAssertEqual(one?.id, "req-7")
    }

    func testAnAnswerNobodyWaitsForIsDropped() async throws {
        let h = await pairedHarness(caps: described)
        h.mac.historyExchanges["req-7"] = exchange("req-7")
        await h.mac.sendDown(HistoryExchange(reqId: "stray", id: "req-7", item: nil), to: h.mac.pairedPhones[0])
        let item = try await h.engine.sessionExchange(macId: h.mac.macId, id: "req-7")
        XCTAssertEqual(item?.id, "req-7")
    }

    // MARK: failing

    func testNoAnswerIsATimeoutAndTheNextReadIsServed() async throws {
        let h = await pairedHarness(caps: described)
        h.mac.historyExchanges["req-7"] = exchange("req-7")
        h.mac.ignoreHistoryExchange = true
        let macId = h.mac.macId
        let task = Task { try await h.engine.sessionExchange(macId: macId, id: "req-7") }
        await eventually("the Mac to get the request") { h.mac.count(HistoryExchangeGet.type) == 1 }
        await h.pass(seconds: Int(PhoneEngine.todoReadTimeout) + 2)
        do {
            _ = try await task.value
            XCTFail("timeout")
        } catch BackendError.timedOut {}

        h.mac.ignoreHistoryExchange = false
        let item = try await h.engine.sessionExchange(macId: macId, id: "req-7")
        XCTAssertEqual(item?.id, "req-7")
    }

    func testAMacThatGoesAwayFailsTheReadAtOnce() async throws {
        let h = await pairedHarness(caps: described)
        h.mac.ignoreHistoryExchange = true
        let macId = h.mac.macId
        let task = Task { try await h.engine.sessionExchange(macId: macId, id: "req-7") }
        await eventually("the Mac to get the request") { h.mac.count(HistoryExchangeGet.type) == 1 }
        await h.mac.publishPresence(online: false)
        do {
            _ = try await task.value
            XCTFail("offline")
        } catch BackendError.macOffline {}
    }
}

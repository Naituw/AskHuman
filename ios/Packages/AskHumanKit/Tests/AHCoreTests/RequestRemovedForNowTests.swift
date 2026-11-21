import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// "Remove for now" (docs/specs/ios-offline-remove.md): while its Mac is offline a request can be taken off the lists of the
/// phone. It is not a cancellation, so nothing goes to the Mac and the Agent keeps waiting. The request is still there for
/// the phone, with its draft; when the Mac is back and still has it, it returns.
final class RequestRemovedForNowTests: XCTestCase {
    private func delivered(_ h: Harness, foreground: Bool = false) async throws -> RequestView {
        await h.pair()
        if foreground { await h.engine.setForeground(true) }
        let request = sampleAsk()
        await h.mac.deliver(request)
        await eventually("inbox") { await h.state().inbox.count == 1 }
        return request
    }

    private func macGoesAway(_ h: Harness) async {
        h.mac.client.drop()
        await eventually("macOffline") { await h.state().macs.first?.link == .macOffline }
    }

    private func macComesBack(_ h: Harness) async {
        await h.mac.start(password: "pw")
        await eventually("online again") { await h.state().macs.first?.link == .online }
    }

    private func key(_ h: Harness, _ request: RequestView) -> String { CachedRequest.key(h.mac.macId, request.id) }

    private func arrivals(_ h: Harness) -> Int {
        h.events.filter { if case .requestArrived = $0 { return true } else { return false } }.count
    }

    /// A Mac that can be reached is not stuck: the user cancels the request there, and the Agent is told.
    func testNothingIsRemovedWhileTheMacIsOnline() async throws {
        let h = Harness()
        let request = try await delivered(h)

        await h.engine.removeRequestForNow(macId: h.mac.macId, requestId: request.id)

        let state = await h.state()
        XCTAssertEqual(state.inbox.map(\.requestId), [request.id])
        XCTAssertTrue(state.removedForNowIds.isEmpty)
        XCTAssertFalse(h.store.cachedRequests().contains { $0.removedForNow })
    }

    func testARemovedRequestIsOffEveryListAndNothingIsSent() async throws {
        let h = Harness()
        let request = try await delivered(h)
        await macGoesAway(h)

        await h.engine.removeRequestForNow(macId: h.mac.macId, requestId: request.id)

        let state = await h.state()
        XCTAssertTrue(state.inbox.isEmpty)
        XCTAssertEqual(state.removedForNowIds, [key(h, request)])
        XCTAssertEqual(state.macs.first?.pendingCount, 0, "the count of the Mac does not include it")
        XCTAssertEqual(h.store.cachedRequests().first?.removedForNow, true, "the mark is on disk")

        await macComesBack(h)
        await eventually("it returns") { await h.state().inbox.count == 1 }
        XCTAssertEqual(h.mac.count(AnswerSubmit.type), 0, "removing is not cancelling")
        XCTAssertEqual(h.mac.count(ConfirmDecide.type), 0)
        XCTAssertNil(h.mac.answer(for: request.id))
    }

    /// The user reads a request once and the dot is for what is new to the list, which a request that returns is.
    func testItReturnsWithTheMacAtItsPlaceWithADotAndWithoutAnArrival() async throws {
        let h = Harness()
        let request = try await delivered(h, foreground: true)
        var newer = sampleAsk(text: "Newer question?")
        newer.createdAt = request.createdAt + 60_000
        await h.mac.deliver(newer)
        await eventually("two") { await h.state().inbox.count == 2 }
        await h.engine.markSeen(macId: h.mac.macId, requestId: request.id)
        await eventually("both arrivals") { self.arrivals(h) == 2 }
        let arrived = arrivals(h)
        var state = await h.state()
        XCTAssertEqual(state.inbox.map(\.requestId), [newer.id, request.id], "the newer first")
        XCTAssertEqual(state.inbox.last?.unread, false)

        await macGoesAway(h)
        await h.engine.removeRequestForNow(macId: h.mac.macId, requestId: request.id)
        state = await h.state()
        XCTAssertEqual(state.inbox.map(\.requestId), [newer.id], "only the one that was removed is off the list")

        await macComesBack(h)
        await eventually("it returns") { await h.state().inbox.count == 2 }
        state = await h.state()
        XCTAssertEqual(state.inbox.map(\.requestId), [newer.id, request.id], "back at its place in the order")
        XCTAssertEqual(state.inbox.last?.unread, true, "new to the list again")
        XCTAssertTrue(state.removedForNowIds.isEmpty)
        XCTAssertEqual(arrivals(h), arrived, "a snapshot never announces an arrival")
        XCTAssertFalse(h.store.cachedRequests().contains { $0.removedForNow })
    }

    /// The request ended while the Mac was away (the Agent gave up, or somebody answered elsewhere): the snapshot does not
    /// list it, and it is finished the way a request that ended in an absence always is.
    func testItDoesNotReturnWhenItEndedMeanwhile() async throws {
        let h = Harness()
        let request = try await delivered(h)
        h.store.saveDraft(Draft(answers: [DraftAnswer(selected: [1], text: "half")]), key: key(h, request))
        await macGoesAway(h)
        await h.engine.removeRequestForNow(macId: h.mac.macId, requestId: request.id)
        await h.mac.resolve(request.id)

        await macComesBack(h)
        await eventually("finished") { await h.engine.history().count == 1 }

        let state = await h.state()
        XCTAssertTrue(state.inbox.isEmpty)
        XCTAssertTrue(state.removedForNowIds.isEmpty)
        let entry = try unwrap(await h.engine.history().first)
        XCTAssertEqual(entry.outcome, .ended)
        XCTAssertNil(h.store.draft(key: key(h, request)), "the draft goes with a request that ended")
        XCTAssertTrue(h.store.cachedRequests().isEmpty)
    }

    func testTheDraftStaysWhileTheRequestIsRemovedAndAfterItReturns() async throws {
        let h = Harness()
        let request = try await delivered(h)
        let draft = Draft(answers: [DraftAnswer(selected: [1], text: "half way")])
        h.store.saveDraft(draft, key: key(h, request))
        await macGoesAway(h)

        await h.engine.removeRequestForNow(macId: h.mac.macId, requestId: request.id)
        XCTAssertEqual(h.store.draft(key: key(h, request))?.answers, draft.answers)

        await macComesBack(h)
        await eventually("it returns") { await h.state().inbox.count == 1 }
        XCTAssertEqual(h.store.draft(key: key(h, request))?.answers, draft.answers)
    }

    func testItStaysRemovedAcrossARestartUntilTheMacIsBack() async throws {
        let h = Harness()
        let request = try await delivered(h)
        await macGoesAway(h)
        await h.engine.removeRequestForNow(macId: h.mac.macId, requestId: request.id)
        await h.engine.setForeground(false)

        let engine2 = PhoneEngine(
            vault: h.vault, store: h.store, broker: h.broker.makeClient(), clock: h.clock, relay: FakeRelay(),
            config: EngineConfig(
                deviceName: "Test iPhone", appVersion: "1.0", bundleId: "b", pushEnv: "dev", allowPlainLoopback: false,
                assetDirectory: h.root.appendingPathComponent("assets2")
            )
        )
        await engine2.start()
        var state = await engine2.snapshot()
        XCTAssertTrue(state.inbox.isEmpty, "still removed after the restart")
        XCTAssertEqual(state.removedForNowIds, [key(h, request)])

        await engine2.setForeground(true)
        await h.mac.start(password: "pw")
        await eventually("it returns in the new session") { await engine2.snapshot().inbox.count == 1 }
        state = await engine2.snapshot()
        XCTAssertTrue(state.removedForNowIds.isEmpty)
    }

    /// An answer that was sent (and not acknowledged) is the user's, whatever the request looks like on the phone: it goes
    /// out again when the Mac is back, the request ends, and nothing is left on the phone.
    func testASubmissionThatWasWaitingIsStillDeliveredWhenTheMacIsBack() async throws {
        let h = Harness()
        let request = try await delivered(h)
        h.mac.ignoreAnswers = true
        let id = try await h.engine.submitAnswer(
            macId: h.mac.macId, requestId: request.id, action: "send", answers: [AnswerItem(q: 0, sel: [0])]
        )
        await eventually("first try") { h.mac.count(AnswerSubmit.type) == 1 }
        await macGoesAway(h)
        await h.advance(until: {
            if case .failed = await h.state().firstInbox?.state { return true }
            return false
        }, "the acknowledgement timeout")

        await h.engine.removeRequestForNow(macId: h.mac.macId, requestId: request.id)
        var state = await h.state()
        XCTAssertTrue(state.inbox.isEmpty)
        XCTAssertEqual(h.store.outboxItems().map(\.clientMsgId), [id], "the submission is not touched")

        h.mac.ignoreAnswers = false
        await macComesBack(h)
        await eventually("the answer is accepted") { await h.engine.history().count == 1 }

        state = await h.state()
        XCTAssertTrue(state.inbox.isEmpty)
        XCTAssertTrue(state.removedForNowIds.isEmpty)
        XCTAssertEqual(h.mac.answer(for: request.id)?.clientMsgId, id)
        let entry = try unwrap(await h.engine.history().first)
        XCTAssertEqual(entry.outcome, .answered)
        XCTAssertTrue(h.store.outboxItems().isEmpty)
    }

    /// Unpairing is still the way to forget a Mac for good, and it takes what was removed along with the rest.
    func testUnpairingTakesTheRemovedRequestAndItsDraftToo() async throws {
        let h = Harness()
        let request = try await delivered(h)
        h.store.saveDraft(Draft(answers: [DraftAnswer(text: "kept")]), key: key(h, request))
        await macGoesAway(h)
        await h.engine.removeRequestForNow(macId: h.mac.macId, requestId: request.id)

        await h.engine.unpair(macId: h.mac.macId)

        let state = await h.state()
        XCTAssertTrue(state.removedForNowIds.isEmpty)
        XCTAssertTrue(h.store.cachedRequests().isEmpty)
        XCTAssertNil(h.store.draft(key: key(h, request)))
    }

    /// A second tap, or a request that is not there: nothing happens.
    func testRemovingTwiceOrAnUnknownRequestChangesNothing() async throws {
        let h = Harness()
        let request = try await delivered(h)
        await macGoesAway(h)

        await h.engine.removeRequestForNow(macId: h.mac.macId, requestId: "nope")
        await h.engine.removeRequestForNow(macId: "nobody", requestId: request.id)
        var state = await h.state()
        XCTAssertEqual(state.inbox.count, 1)
        XCTAssertTrue(state.removedForNowIds.isEmpty)

        await h.engine.removeRequestForNow(macId: h.mac.macId, requestId: request.id)
        await h.engine.removeRequestForNow(macId: h.mac.macId, requestId: request.id)
        state = await h.state()
        XCTAssertEqual(state.removedForNowIds, [key(h, request)])
    }
}

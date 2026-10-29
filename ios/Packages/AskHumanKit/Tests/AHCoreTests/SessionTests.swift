import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

final class SessionTests: XCTestCase {
    // MARK: handshake

    func testHelloAckWithTheWrongNonceIsIgnored() async throws {
        let h = Harness()
        await h.mac.start(password: "pw")
        h.mac.autoHello = false
        await h.engine.start()
        let code = h.mac.startPairing(password: "pw")
        await h.engine.startPairing(code: code)
        await eventually("knock") { h.mac.sas != nil }
        await h.mac.allow()
        await eventually("hello") { h.mac.count(Wire.T.hello) == 1 }

        // An attacker (or an old session) answers with a nonce the phone never sent.
        await h.mac.sendHelloSync(to: h.mac.pairedPhones[0], nonce: "forged")
        try await Task.sleep(nanoseconds: 50_000_000)
        var link = await h.state().macs.first?.link
        XCTAssertNotEqual(link, .online)

        let nonce = try h.mac.received(Wire.T.hello)[0].body(as: Hello.self).nonce
        await h.mac.sendHelloSync(to: h.mac.pairedPhones[0], nonce: nonce)
        await eventually("online") { await h.state().macs.first?.link == .online }
        link = await h.state().macs.first?.link
        XCTAssertEqual(link, .online)
    }

    /// Pairs with a Mac that does not answer `hello` until the test lets it; returns once the first
    /// hello is out.
    private func pairWithASilentMac(_ h: Harness) async {
        await h.mac.start(password: "pw")
        h.mac.autoHello = false
        await h.engine.start()
        await h.engine.startPairing(code: h.mac.startPairing(password: "pw"))
        await eventually("knock") { h.mac.sas != nil }
        await h.mac.allow()
        await eventually("first hello") { h.mac.count(Wire.T.hello) == 1 }
    }

    /// The nonce of the n-th hello the Mac received (counting from 0).
    private func helloNonce(_ h: Harness, _ index: Int) throws -> String {
        try h.mac.received(Wire.T.hello)[index].body(as: Hello.self).nonce
    }

    func testHelloIsRetriedInRoundsThatGetSlower() async throws {
        let h = Harness()
        await pairWithASilentMac(h)
        await h.advance(until: { h.mac.count(Wire.T.hello) == 2 }, "second hello")
        await h.advance(until: { h.mac.count(Wire.T.hello) == 3 }, "third hello")

        // The first round is over after 15 seconds and the next one starts 30 seconds later.
        await h.pass(seconds: 20)
        XCTAssertEqual(h.mac.count(Wire.T.hello), 3, "between the rounds the phone leaves the Mac alone")
        await h.advance(until: { h.mac.count(Wire.T.hello) == 4 }, limit: 40, "the second round")
        await h.advance(until: { h.mac.count(Wire.T.hello) == 6 }, "the end of the second round")

        // A round that failed again is followed by a longer wait: a minute.
        await h.pass(seconds: 50)
        XCTAssertEqual(h.mac.count(Wire.T.hello), 6)
        await h.advance(until: { h.mac.count(Wire.T.hello) == 7 }, limit: 30, "the third round")
        let state = await h.state()
        XCTAssertNotEqual(state.macs.first?.link, .online)
    }

    func testTheSlowRetriesEndWithTheFirstAnswer() async throws {
        let h = Harness()
        await pairWithASilentMac(h)
        await h.advance(until: { h.mac.count(Wire.T.hello) == 3 }, "third hello")
        await h.pass(seconds: 20)
        await h.advance(until: { h.mac.count(Wire.T.hello) == 4 }, limit: 40, "the second round")

        h.mac.autoHello = true
        await h.advance(until: { await h.state().macs.first?.link == .online }, "an answer to the next hello")
        let hellos = h.mac.count(Wire.T.hello)
        await h.pass(seconds: 300)
        XCTAssertEqual(h.mac.count(Wire.T.hello), hellos, "once it answered, nothing asks again")
    }

    func testAnAnswerToAnEarlierHelloStillCompletesTheHandshake() async throws {
        let h = Harness()
        await pairWithASilentMac(h)
        await h.advance(until: { h.mac.count(Wire.T.hello) == 2 }, "second hello")

        // A slow Mac: its answer to the first hello arrives after the phone has sent the second.
        await h.mac.sendHelloSync(to: h.mac.pairedPhones[0], nonce: try helloNonce(h, 0))
        await eventually("online") { await h.state().macs.first?.link == .online }

        // The answer to the second one comes later still. The handshake is done, so it changes nothing:
        // with the epoch it carries here it would break the session if it counted.
        let late = HelloAck(
            nonce: try helloNonce(h, 1), epoch: "bogus", macName: "Home Mac", caps: h.mac.caps,
            slot: SlotInfo(active: true, autoActivation: false)
        )
        await h.mac.sendDown(late, to: h.mac.pairedPhones[0])
        await h.mac.deliver(sampleAsk())
        await eventually("a request that still arrives live") { await h.state().inbox.count == 1 }
    }

    func testAnAnswerThatComesAfterTheRoundWasGivenUpIsAccepted() async throws {
        let h = Harness()
        await pairWithASilentMac(h)
        await h.advance(until: { h.mac.count(Wire.T.hello) == 3 }, "third hello")
        await h.pass(seconds: 8)
        var state = await h.state()
        XCTAssertNotEqual(state.macs.first?.link, .online)

        await h.mac.sendHelloSync(to: h.mac.pairedPhones[0], nonce: try helloNonce(h, 2))
        await eventually("online") { await h.state().macs.first?.link == .online }
        state = await h.state()
        XCTAssertEqual(state.macs.first?.link, .online)
        // The slow retry that was waiting is gone.
        await h.pass(seconds: 100)
        XCTAssertEqual(h.mac.count(Wire.T.hello), 3)
    }

    func testAnAnswerToAHelloOfAnEarlierSessionIsIgnored() async throws {
        let h = Harness()
        await pairWithASilentMac(h)
        let old = try helloNonce(h, 0)

        // The Mac goes away and comes back: a new session, in which the old nonce means nothing.
        await h.mac.publishPresence(online: false)
        await eventually("offline") { await h.state().macs.first?.link == .macOffline }
        await h.mac.publishPresence(online: true)
        await eventually("a hello of the new session") { h.mac.count(Wire.T.hello) == 2 }
        await h.mac.sendHelloSync(to: h.mac.pairedPhones[0], nonce: old)
        try await Task.sleep(nanoseconds: 50_000_000)
        let state = await h.state()
        XCTAssertNotEqual(state.macs.first?.link, .online)
    }

    func testPullToRefreshStartsAHandshakeThatNeverFinishedOver() async throws {
        let h = Harness()
        await pairWithASilentMac(h)
        await h.advance(until: { h.mac.count(Wire.T.hello) == 3 }, "third hello")
        await h.pass(seconds: 8)
        var state = await h.state()
        XCTAssertNotEqual(state.macs.first?.link, .online)

        // The Mac is reachable again; the user pulls the list down. No waiting for the next round.
        h.mac.autoHello = true
        await h.engine.refresh()
        await eventually("online without advancing the clock") { await h.state().macs.first?.link == .online }
        state = await h.state()
        XCTAssertEqual(state.macs.first?.link, .online)
        XCTAssertEqual(h.mac.count(Wire.T.hello), 4)
    }

    func testALinkCheckStartsAHandshakeThatNeverFinishedOverWithoutReconnecting() async throws {
        let h = Harness()
        await pairWithASilentMac(h)
        await h.advance(until: { h.mac.count(Wire.T.hello) == 3 }, "third hello")
        await h.pass(seconds: 8)
        let connections = h.phone.attempts.count

        // A request notification reaches the open app, and the phone's inbox is empty.
        h.mac.autoHello = true
        await h.engine.checkLink(macId: h.mac.macId)
        await eventually("online without advancing the clock") { await h.state().macs.first?.link == .online }
        XCTAssertEqual(h.mac.count(Wire.T.hello), 4)
        // The probe came back: the connection was fine all along and is not replaced.
        await h.pass(seconds: Int(PhoneEngine.probeTimeout) + 2)
        XCTAssertEqual(h.phone.attempts.count, connections)
        let state = await h.state()
        XCTAssertEqual(state.macs.first?.link, .online)
    }

    func testRefreshAsksEveryReachableMacForAFreshInboxSnapshot() async throws {
        let h = Harness()
        await h.pair()
        let before = h.mac.received(Wire.T.resync).filter { (try? $0.body(as: Resync.self).stream) == "inbox" }.count
        await h.engine.refresh()
        await eventually("an inbox resync") {
            h.mac.received(Wire.T.resync).filter { (try? $0.body(as: Resync.self).stream) == "inbox" }.count == before + 1
        }
    }

    func testRefreshReconnectsWhenTheLinkIsWaitingToRetry() async throws {
        let h = Harness()
        await h.pair()
        h.phone.refusedKinds = [.tls, .wss]
        h.phone.drop()
        await eventually("broker down") { await h.state().macs.first?.link == .brokerDown }
        await h.advance(until: {
            if case .backoff = await h.state().phase { return true }
            return false
        }, "a backoff after the failed reconnect")

        // The user pulls to refresh once the network is back: no waiting for the backoff timer.
        h.phone.refusedKinds = []
        await h.engine.refresh()
        await eventually("online again without advancing the clock") { await h.state().macs.first?.link == .online }
    }

    func testANewEpochTriggersANewHandshake() async throws {
        let h = Harness()
        await h.pair()
        XCTAssertEqual(h.mac.count(Wire.T.hello), 1)
        // The gateway restarted: same Mac, new epoch in its presence.
        h.mac.epoch = "restarted"
        await h.mac.publishPresence(online: true)
        await eventually("second hello") { h.mac.count(Wire.T.hello) == 2 }
        await eventually("still online") { await h.state().macs.first?.link == .online }
        // Snapshots of the old epoch are not accepted any more, those of the new one are.
        let request = sampleAsk()
        await h.mac.deliver(request)
        await eventually("request of the new epoch") { await h.state().inbox.count == 1 }
    }

    func testEpochChangeWithTheSameKeysResyncs() async throws {
        let h = Harness()
        await h.pair()
        let request = sampleAsk()
        await h.mac.deliver(request)
        await eventually("request") { await h.state().inbox.count == 1 }

        // Simulate a restart of the gateway: new presence epoch, inbox gone.
        await h.mac.publishPresence(online: false)
        await eventually("offline") { await h.state().macs.first?.link == .macOffline }
        await h.mac.resolve(request.id, how: "cancelled", by: nil)
        await h.mac.publishPresence(online: true)
        await eventually("back online") { await h.state().macs.first?.link == .online }
        // The request ended while the phone was away: the snapshot clears it.
        await eventually("request cleared") { await h.state().inbox.isEmpty }
        let history = await h.engine.history()
        XCTAssertEqual(history.first?.id, "\(h.mac.macId)/\(request.id)")
    }

    // MARK: requests

    func testRequestNewIsStoredAcknowledgedAndShown() async throws {
        let h = Harness()
        await h.pair()
        let request = sampleAsk()
        await h.mac.deliver(request)
        await eventually("inbox") { await h.state().inbox.count == 1 }
        let item = try unwrap(await h.state().firstInbox)
        XCTAssertEqual(item.requestId, request.id)
        XCTAssertEqual(item.view, request)
        XCTAssertTrue(item.unread)
        XCTAssertEqual(item.macName, "Home Mac")
        await eventually("request.ack") { h.mac.count(RequestAck.type) == 1 }
        XCTAssertEqual(try h.mac.received(RequestAck.type)[0].body(as: RequestAck.self).requestId, request.id)
        XCTAssertEqual(h.store.cachedRequests().count, 1, "persisted")
        XCTAssertEqual(h.events.contains(.requestArrived(macId: h.mac.macId, requestId: request.id)), true)

        await h.engine.markSeen(macId: h.mac.macId, requestId: request.id)
        await eventually("request.seen") { h.mac.count(RequestSeen.type) == 1 }
        let unread = await h.state().firstInbox?.unread
        XCTAssertEqual(unread, false)
        await h.engine.markSeen(macId: h.mac.macId, requestId: request.id)
        try await Task.sleep(nanoseconds: 30_000_000)
        XCTAssertEqual(h.mac.count(RequestSeen.type), 1, "seen is sent once")
    }

    func testALightRequestIsCompletedWithRequestGet() async throws {
        let h = Harness()
        await h.pair()
        let request = sampleAsk(text: "long body")
        await h.mac.deliver(request, full: false)
        await eventually("request.get") { h.mac.count(RequestGet.type) == 1 }
        await eventually("full view") { await h.state().firstInbox?.view != nil }
        await eventually("ack after the full view") { h.mac.count(RequestAck.type) == 1 }
    }

    func testTheSessionTitleOfARequestReachesTheInboxAndStaysInTheHistory() async throws {
        let h = Harness()
        await h.pair()
        let request = sampleAsk(sessionTitle: "Refactor the gateway")
        await h.mac.deliver(request)
        await eventually("inbox") { await h.state().inbox.count == 1 }
        var item = try unwrap(await h.state().firstInbox)
        XCTAssertEqual(item.summary.sessionTitle, "Refactor the gateway")
        XCTAssertEqual(item.sessionTitle, "Refactor the gateway")
        XCTAssertEqual(item.displayTitle, "Refactor the gateway", "the title names the request, not the Agent")

        await h.mac.resolve(request.id, how: "answeredElsewhere", by: "Telegram")
        await eventually("gone") { await h.state().inbox.isEmpty }
        let entry = try unwrap(await h.engine.history().first)
        XCTAssertEqual(entry.request.sessionTitle, "Refactor the gateway", "the history keeps it")

        // A request without one is named by its Agent.
        await h.mac.deliver(sampleAsk())
        await eventually("second request") { await h.state().inbox.count == 1 }
        item = try unwrap(await h.state().firstInbox)
        XCTAssertNil(item.sessionTitle)
        XCTAssertEqual(item.displayTitle, "Codex")
    }

    func testABlankSessionTitleCountsAsNone() async throws {
        let h = Harness()
        await h.pair()
        await h.mac.deliver(sampleAsk(sessionTitle: " \n "))
        await eventually("inbox") { await h.state().inbox.count == 1 }
        let item = try unwrap(await h.state().firstInbox)
        XCTAssertNil(item.sessionTitle)
        XCTAssertEqual(item.displayTitle, "Codex")
    }

    func testARequestThatCameWithTheSnapshotKnowsItsSessionTitleBeforeItsFullViewIsHere() async throws {
        let h = Harness()
        await h.pair()
        await h.mac.deliver(sampleAsk())
        await eventually("first") { await h.state().inbox.count == 1 }
        // The second request's delta is lost: it reaches the phone only as a line of the inbox snapshot.
        h.mac.skipRevision()
        await h.mac.deliver(sampleAsk(sessionTitle: "Fix the flaky login test"))
        await eventually("snapshot brought the second request") { await h.state().inbox.count == 2 }
        let titled = try unwrap(await h.state().inbox.first { $0.sessionTitle != nil })
        XCTAssertEqual(titled.summary.sessionTitle, "Fix the flaky login test")
        XCTAssertEqual(titled.sessionTitle, "Fix the flaky login test")
    }

    func testDuplicateAndOldDeltasAreIgnoredAndGapsResync() async throws {
        let h = Harness()
        await h.pair()
        let first = sampleAsk()
        await h.mac.deliver(first)
        await eventually("first") { await h.state().inbox.count == 1 }

        // A delta the phone missed: the next one has a gap, so the phone asks for the snapshot.
        h.mac.skipRevision()
        let second = sampleAsk()
        await h.mac.deliver(second)
        await eventually("resync") { h.mac.received(Resync.type).count == 1 }
        XCTAssertEqual(try h.mac.received(Resync.type)[0].body(as: Resync.self).stream, "inbox")
        await eventually("snapshot brought the second request") { await h.state().inbox.count == 2 }
    }

    func testResolvedByAnotherChannelMovesTheRequestToHistory() async throws {
        let h = Harness()
        await h.pair()
        let request = sampleAsk()
        await h.mac.deliver(request)
        await eventually("inbox") { await h.state().inbox.count == 1 }
        await h.mac.resolve(request.id, how: "answeredElsewhere", by: "Telegram")
        await eventually("gone") { await h.state().inbox.isEmpty }
        let entry = try unwrap(await h.engine.history().first)
        XCTAssertEqual(entry.outcome, .answeredElsewhere)
        XCTAssertEqual(entry.by, "Telegram")
        XCTAssertEqual(entry.request, request)
        XCTAssertEqual(h.store.cachedRequests().count, 0)
        XCTAssertEqual(h.events.contains(.requestFinished(macId: h.mac.macId, requestId: request.id, outcome: .answeredElsewhere, by: "Telegram")), true)
    }

    // MARK: answers

    func testAnswerFlowAcceptedByTheMac() async throws {
        let h = Harness()
        await h.pair()
        let request = sampleAsk()
        await h.mac.deliver(request)
        await eventually("inbox") { await h.state().inbox.count == 1 }

        let answers = [AnswerItem(q: 0, sel: [1], text: "because")]
        let id = try await h.engine.submitAnswer(macId: h.mac.macId, requestId: request.id, action: "send", answers: answers)
        await eventually("finished") { await h.state().inbox.isEmpty }
        let seen = try XCTUnwrap(h.mac.answer(for: request.id))
        XCTAssertEqual(seen.clientMsgId, id)
        XCTAssertEqual(seen.answers, answers)
        XCTAssertEqual(seen.action, "send")

        let entry = try unwrap(await h.engine.history().first)
        XCTAssertEqual(entry.outcome, .answered)
        XCTAssertEqual(entry.answers, answers, "the history keeps what was sent, even if the resolution came first")
        XCTAssertTrue(h.store.outboxItems().isEmpty)
    }

    func testCancelActionIsRecordedAsCancelled() async throws {
        let h = Harness()
        await h.pair()
        let request = sampleAsk()
        await h.mac.deliver(request)
        await eventually("inbox") { await h.state().inbox.count == 1 }
        _ = try await h.engine.submitAnswer(macId: h.mac.macId, requestId: request.id, action: "cancel", answers: [])
        await eventually("finished") { await h.state().inbox.isEmpty }
        XCTAssertEqual(h.mac.answer(for: request.id)?.action, "cancel")
    }

    func testAnswerTimeoutFailsThenRetryReusesTheClientMessageId() async throws {
        let h = Harness()
        await h.pair()
        let request = sampleAsk()
        await h.mac.deliver(request)
        await eventually("inbox") { await h.state().inbox.count == 1 }
        h.mac.ignoreAnswers = true

        let id = try await h.engine.submitAnswer(macId: h.mac.macId, requestId: request.id, action: "send", answers: [AnswerItem(q: 0, sel: [0])])
        var item = try unwrap(await h.state().firstInbox)
        if case .sending(let sending, _) = item.state { XCTAssertEqual(sending, id) } else { XCTFail("expected sending, got \(item.state)") }

        await h.advance(until: {
            if case .failed = await h.state().firstInbox?.state { return true }
            return false
        }, "the acknowledgement timeout")
        item = try unwrap(await h.state().firstInbox)
        XCTAssertEqual(item.state, .failed(clientMsgId: id, reason: "timeout"))
        XCTAssertEqual(h.store.outboxItems().count, 1, "kept for the retry")

        h.mac.ignoreAnswers = false
        try await h.engine.retrySubmission(clientMsgId: id)
        await eventually("finished") { await h.state().inbox.isEmpty }
        XCTAssertEqual(h.mac.answer(for: request.id)?.clientMsgId, id)
        XCTAssertEqual(h.mac.count(AnswerSubmit.type), 2, "sent twice under one id")
    }

    func testLostAndExpiredAcknowledgementsFinishTheRequest() async throws {
        for (status, expected) in [("lost", HistoryOutcome.answeredElsewhere), ("expired", .expired)] {
            let h = Harness()
            await h.pair()
            let request = sampleAsk()
            await h.mac.deliver(request)
            await eventually("inbox") { await h.state().inbox.count == 1 }
            h.mac.answerStatus = status
            _ = try await h.engine.submitAnswer(macId: h.mac.macId, requestId: request.id, action: "send", answers: [AnswerItem(q: 0, sel: [0])])
            await eventually("finished (\(status))") { await h.state().inbox.isEmpty }
            let outcome = await h.engine.history().first?.outcome
            XCTAssertEqual(outcome, expected, status)
        }
    }

    func testInvalidAnswerStaysInTheInboxAsFailedAndCanBeDiscarded() async throws {
        let h = Harness()
        await h.pair()
        let request = sampleAsk()
        await h.mac.deliver(request)
        await eventually("inbox") { await h.state().inbox.count == 1 }
        h.mac.answerStatus = "invalid"
        let id = try await h.engine.submitAnswer(macId: h.mac.macId, requestId: request.id, action: "send", answers: [AnswerItem(q: 0, sel: [0])])
        await eventually("failed") {
            if case .failed = await h.state().firstInbox?.state { return true }
            return false
        }
        let state = await h.state().firstInbox?.state
        XCTAssertEqual(state, .failed(clientMsgId: id, reason: "bad answer"))
        XCTAssertTrue(h.store.outboxItems().isEmpty, "an invalid answer cannot succeed on retry")
        await h.engine.discardSubmission(clientMsgId: id)
        let after = await h.state().firstInbox?.state
        XCTAssertEqual(after, .pending)
    }

    func testSubmitWhileTheMacIsOfflineThrowsAndQueuesNothing() async throws {
        let h = Harness()
        await h.pair()
        let request = sampleAsk()
        await h.mac.deliver(request)
        await eventually("inbox") { await h.state().inbox.count == 1 }
        await h.mac.publishPresence(online: false)
        await eventually("offline") { await h.state().macs.first?.link == .macOffline }
        do {
            _ = try await h.engine.submitAnswer(macId: h.mac.macId, requestId: request.id, action: "send", answers: [])
            XCTFail("expected an error")
        } catch {
            XCTAssertEqual(error as? BackendError, .macOffline)
        }
        XCTAssertTrue(h.store.outboxItems().isEmpty)
        let pending = await h.state().inbox.count
        XCTAssertEqual(pending, 1, "the request stays in the inbox")
    }

    func testConfirmDecisionRoundTrip() async throws {
        let h = Harness()
        await h.pair()
        let request = sampleConfirm()
        await h.mac.deliver(request)
        await eventually("inbox") { await h.state().inbox.count == 1 }
        _ = try await h.engine.submitConfirm(macId: h.mac.macId, requestId: request.id, wireIndex: 0, dismiss: false, input: "ok")
        await eventually("finished") { await h.state().inbox.isEmpty }
        let decide = try h.mac.received(ConfirmDecide.type)[0].body(as: ConfirmDecide.self)
        XCTAssertEqual(decide.wireIndex, 0)
        XCTAssertEqual(decide.input, "ok")
        let entry = try unwrap(await h.engine.history().first)
        XCTAssertEqual(entry.outcome, .answered)
        XCTAssertEqual(entry.confirmChoice, "Allow")
        XCTAssertEqual(entry.confirmInput, "ok")
    }

    func testAnswerAfterRestartIsRecoveredFromTheOutbox() async throws {
        let h = Harness()
        await h.pair()
        let request = sampleAsk()
        await h.mac.deliver(request)
        await eventually("inbox") { await h.state().inbox.count == 1 }
        h.mac.ignoreAnswers = true
        let id = try await h.engine.submitAnswer(macId: h.mac.macId, requestId: request.id, action: "send", answers: [AnswerItem(q: 0, sel: [0])])
        await eventually("answer at the Mac") { h.mac.count(AnswerSubmit.type) == 1 }
        await h.engine.setForeground(false)

        // The app is relaunched: the request is still "sending"; after the handshake the answer goes out again.
        h.mac.ignoreAnswers = false
        let engine2 = PhoneEngine(
            vault: h.vault, store: h.store, broker: h.broker.makeClient(), clock: h.clock, relay: FakeRelay(),
            config: EngineConfig(deviceName: "Test iPhone", appVersion: "1.0", bundleId: "b", pushEnv: "dev", allowPlainLoopback: false, assetDirectory: h.root.appendingPathComponent("assets2"))
        )
        await engine2.start()
        let item = try unwrap(await engine2.snapshot().inbox.first)
        if case .sending(let sending, _) = item.state { XCTAssertEqual(sending, id) } else { XCTFail("expected sending, got \(item.state)") }
        await engine2.setForeground(true)
        await eventually("the answer is accepted after the retry") { await engine2.snapshot().inbox.isEmpty }
        XCTAssertEqual(h.mac.count(AnswerSubmit.type), 2)
        XCTAssertEqual(h.mac.answer(for: request.id)?.clientMsgId, id)
    }

    // MARK: clocks and tampering

    func testMessagesWithAStaleClockAreDroppedAndFlagged() async throws {
        let h = Harness()
        await h.pair()
        let stale = Plain(
            t: RequestNew.type, ts: h.clock.nowMs() - Wire.clockSkewMs - 60_000,
            b: try AHJSON.tree(RequestNew(rev: 99, request: sampleAsk(), full: true))
        )
        await h.mac.sendRaw(plain: stale, to: h.mac.pairedPhones[0])
        await eventually("clock skew flag") { await h.state().clockSkew }
        let items = await h.state().inbox.count
        XCTAssertEqual(items, 0)
    }

    func testMessagesWithTheWrongKeyAreDroppedSilently() async throws {
        let h = Harness()
        await h.pair()
        let forged = Plain(t: RequestNew.type, ts: h.clock.nowMs(), b: try AHJSON.tree(RequestNew(rev: 99, request: sampleAsk(), full: true)))
        await h.mac.sendRaw(plain: forged, to: h.mac.pairedPhones[0], key: Bytes.random(32))
        try await Task.sleep(nanoseconds: 50_000_000)
        let state = await h.state()
        XCTAssertEqual(state.inbox.count, 0)
        XCTAssertFalse(state.clockSkew)
    }

    func testTheSameMessageIdIsHandledOnce() async throws {
        let h = Harness()
        await h.pair()
        let request = sampleAsk()
        let plain = Plain(t: RequestNew.type, ts: h.clock.nowMs(), b: try AHJSON.tree(RequestNew(rev: 1, request: request, full: true)))
        await h.mac.sendRaw(plain: plain, to: h.mac.pairedPhones[0])
        await h.mac.sendRaw(plain: plain, to: h.mac.pairedPhones[0])
        await eventually("one item") { await h.state().inbox.count == 1 }
        try await Task.sleep(nanoseconds: 50_000_000)
        XCTAssertEqual(h.mac.count(RequestAck.type), 1)
    }

    // MARK: unpairing

    func testUnpairedByTheMacRemovesEverything() async throws {
        let h = Harness()
        await h.pair()
        let request = sampleAsk()
        await h.mac.deliver(request)
        await eventually("inbox") { await h.state().inbox.count == 1 }
        await h.mac.sendDown(Unpaired(), to: h.mac.pairedPhones[0])
        await eventually("removed") { await h.state().macs.isEmpty }
        XCTAssertTrue(h.vault.pairings().isEmpty)
        XCTAssertTrue(h.store.cachedRequests().isEmpty)
        let inbox = await h.state().inbox
        XCTAssertTrue(inbox.isEmpty)
        XCTAssertEqual(h.events.contains(.macUnpaired(macId: h.mac.macId, macName: "Home Mac")), true)
    }

    func testUnpairByTheUserTellsTheMac() async throws {
        let h = Harness()
        await h.pair()
        await h.engine.unpair(macId: h.mac.macId)
        await eventually("unpair message") { h.mac.count(Wire.T.unpair) == 1 }
        XCTAssertTrue(h.vault.pairings().isEmpty)
        let macs = await h.state().macs
        XCTAssertTrue(macs.isEmpty)
    }

    func testForgettingEverythingTellsTheMacsAndClearsTheBrokerAccount() async throws {
        let h = Harness()
        await h.pair()
        await h.mac.deliver(sampleAsk())
        await eventually("inbox") { await h.state().inbox.count == 1 }
        XCTAssertNotNil(h.vault.broker)

        await h.engine.forgetAll()
        await eventually("unpair message") { h.mac.count(Wire.T.unpair) == 1 }
        XCTAssertTrue(h.vault.pairings().isEmpty)
        XCTAssertNil(h.vault.broker)
        XCTAssertNil(h.vault.push)
        XCTAssertTrue(h.store.cachedRequests().isEmpty)
        let state = await h.state()
        XCTAssertTrue(state.macs.isEmpty)
        XCTAssertTrue(state.inbox.isEmpty)
        XCTAssertNil(state.brokerHost)
        XCTAssertEqual(state.phase, .idle)
    }

    func testRenamingAMacKeepsItsRealName() async throws {
        let h = Harness()
        await h.pair()
        await h.engine.renameMac(macId: h.mac.macId, alias: "  Work  ")
        var mac = try unwrap(await h.state().macs.first)
        XCTAssertEqual(mac.name, "Work")
        XCTAssertEqual(mac.macName, "Home Mac")
        XCTAssertEqual(h.vault.pairing(macId: h.mac.macId)?.alias, "Work")
        await h.engine.renameMac(macId: h.mac.macId, alias: nil)
        mac = try unwrap(await h.state().macs.first)
        XCTAssertEqual(mac.name, "Home Mac")
    }
}

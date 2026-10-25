import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

final class ConnectionTests: XCTestCase {
    func testForegroundPresenceIsPublishedOnceTheFirstMacIsInSync() async throws {
        let h = Harness()
        await h.pair()
        await eventually("fg presence") { h.presence(of: h.phoneId)?.fg == 1 }
        XCTAssertEqual(h.presence(of: h.phoneId)?.r, "phone")
        XCTAssertEqual(h.presence(of: h.phoneId)?.o, 1)
    }

    func testForegroundPresenceWaitsForTheSettlePeriodWhenNoMacIsOnline() async throws {
        let h = Harness()
        await h.pair()
        await h.mac.publishPresence(online: false)
        await eventually("offline") { await h.state().macs.first?.link == .macOffline }
        await h.engine.setForeground(false)
        await h.engine.setForeground(true)
        await eventually("connected without a Mac") { h.phone.isConnected && h.presence(of: h.phoneId)?.fg == 0 }
        // The retained "offline" of the Mac is an answer: no grace period before saying so.
        await eventually("macOffline") { await h.state().macs.first?.link == .macOffline }
        await h.advance(until: { h.presence(of: h.phoneId)?.fg == 1 }, step: 0.5, limit: 10, "fg after the settle period")
    }

    func testGoingToTheBackgroundSaysGoodbyeAndComingBackResyncs() async throws {
        let h = Harness()
        await h.pair()
        await eventually("fg") { h.presence(of: h.phoneId)?.fg == 1 }

        await h.engine.setForeground(false)
        XCTAssertEqual(h.presence(of: h.phoneId)?.o, 0)
        XCTAssertEqual(h.presence(of: h.phoneId)?.fg, 0)
        XCTAssertFalse(h.phone.isConnected)

        await h.engine.setForeground(true)
        await eventually("second hello") { h.mac.count(Wire.T.hello) == 2 }
        await eventually("online again") { await h.state().macs.first?.link == .online }
        await eventually("fg again") { h.presence(of: h.phoneId)?.fg == 1 }
    }

    /// Found on a simulator: the permission alert makes the app inactive and active again within
    /// milliseconds. The return found the old link still up (the goodbye was on its way), did nothing, and
    /// the goodbye then dropped the link: the app stayed disconnected in the foreground.
    func testComingBackWhileTheGoodbyeIsOnItsWayStillReconnects() async throws {
        let h = Harness()
        await h.pair()
        await eventually("fg") { h.presence(of: h.phoneId)?.fg == 1 }

        // Hold the goodbye (the offline presence) on its way.
        let gate = Gate()
        let presenceTopic = Wire.topicPresence(user: "u1", deviceId: h.phoneId)
        h.phone.beforePublish = { topic in if topic == presenceTopic { await gate.hold() } }
        let leaving = Task { await h.engine.setForeground(false) }
        await eventually("the goodbye to be on its way") { await gate.isHolding }

        await h.engine.setForeground(true)
        h.phone.beforePublish = nil
        await gate.release()
        await leaving.value

        await eventually("connected again") { h.phone.isConnected }
        await eventually("online again") { await h.state().macs.first?.link == .online }
        await eventually("fg again") { h.presence(of: h.phoneId)?.fg == 1 }
    }

    func testALostConnectionIsRetriedWithBackoffAndResynced() async throws {
        let h = Harness()
        await h.pair()
        await eventually("fg") { h.presence(of: h.phoneId)?.fg == 1 }
        h.phone.drop()
        await eventually("broker down") { await h.state().macs.first?.link == .brokerDown }
        XCTAssertEqual(h.presence(of: h.phoneId)?.o, 0, "the last will announced the departure")

        await h.advance(until: { await h.state().macs.first?.link == .online }, "reconnect and resync")
        XCTAssertEqual(h.mac.count(Wire.T.hello), 2)
        XCTAssertEqual(h.phone.attempts.count, 2)
    }

    func testBackoffGrowsWhileTheBrokerStaysUnreachable() async throws {
        let h = Harness()
        await h.pair()
        h.phone.refusedKinds = [.tls, .wss]
        h.phone.drop()
        await eventually("broker down") { await h.state().macs.first?.link == .brokerDown }
        let before = h.phone.attempts.count
        await h.advance(until: { h.phone.attempts.count >= before + 6 }, step: 1, limit: 400, "several attempts")
        // Each round tries TLS then WSS, and the delays between rounds grow (1, 2, 4 … seconds).
        let state = await h.state()
        XCTAssertEqual(state.macs.first?.link, .brokerDown)
        h.phone.refusedKinds = []
        await h.advance(until: { await h.state().macs.first?.link == .online }, "recovery")
    }

    func testTheMacsDeathIsSeenThroughItsLastWill() async throws {
        let h = Harness()
        await h.pair()
        h.mac.client.drop()
        await eventually("macOffline") { await h.state().macs.first?.link == .macOffline }
        let request = sampleAsk()
        _ = request
        // The Mac comes back and the phone shakes hands again.
        await h.mac.start(password: "pw")
        await eventually("online again") { await h.state().macs.first?.link == .online }
        XCTAssertEqual(h.mac.count(Wire.T.hello), 2)
    }

    func testFallsBackToWebSocketWhenTLSIsBlockedAndRemembersIt() async throws {
        let h = Harness()
        h.phone.refusedKinds = [.tls]
        await h.pair()
        XCTAssertEqual(Array(h.phone.attempts.prefix(2)), [.tls, .wss])
        XCTAssertEqual(h.vault.broker?.lastTransport, "wss")

        await h.engine.setForeground(false)
        await h.engine.setForeground(true)
        await eventually("online") { await h.state().macs.first?.link == .online }
        XCTAssertEqual(h.phone.attempts.count, 3)
        XCTAssertEqual(h.phone.attempts.last, .wss, "the transport that worked is tried first")
    }

    func testTrafficIsCounted() async throws {
        let h = Harness()
        await h.pair()
        await h.mac.deliver(sampleAsk())
        await eventually("inbox") { await h.state().inbox.count == 1 }
        let traffic = try unwrap(await h.state().traffic)
        XCTAssertGreaterThan(traffic.sentMessages, 0)
        XCTAssertGreaterThan(traffic.receivedMessages, 0)
        XCTAssertGreaterThan(traffic.sentBytes, 0)
        XCTAssertGreaterThan(traffic.receivedBytes, 0)
        XCTAssertEqual(traffic.month, PhoneEngine.month(h.clock.now()))
    }

    func testOutgoingMessagesAreRateLimited() async throws {
        let h = Harness()
        await h.pair()
        // Burst 10, then 6 per second: with the clock standing still, only the burst gets through.
        let sender = Task {
            for _ in 0..<25 { await h.engine.send(Ping(t: 0), to: h.mac.macId) }
        }
        try await Task.sleep(nanoseconds: 150_000_000)
        XCTAssertLessThanOrEqual(h.mac.totalReceived, 10, "a burst is at most ten messages")
        XCTAssertLessThan(h.mac.count(Wire.T.ping), 25, "the rest waits for tokens")
        await h.advance(until: { h.mac.count(Wire.T.ping) == 25 }, step: 0.5, limit: 20, "all pings")
        await sender.value
    }
}

/// Holds a caller until the test lets it go.
private actor Gate {
    private(set) var isHolding = false
    private var waiter: CheckedContinuation<Void, Never>?

    func hold() async {
        isHolding = true
        await withCheckedContinuation { waiter = $0 }
    }

    func release() {
        isHolding = false
        waiter?.resume()
        waiter = nil
    }
}

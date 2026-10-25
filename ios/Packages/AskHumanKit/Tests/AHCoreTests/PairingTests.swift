import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

final class PairingTests: XCTestCase {
    func testFullPairingProducesMatchingCodesKeysAndASession() async throws {
        let h = Harness()
        await h.mac.start(password: "pw")
        await h.engine.start()
        let code = h.mac.startPairing(password: "pw")
        await h.engine.startPairing(code: code)

        await eventually("the Mac to see the knock") { h.mac.sas != nil }
        let phoneSas = await h.state().pairing?.sas
        XCTAssertEqual(phoneSas, h.mac.sas, "both sides derive the same six-digit code")
        XCTAssertEqual(h.mac.knockName, "Test iPhone")
        await eventually("pair.pending") { await h.state().pairing?.stage == .macConfirming }

        h.mac.caps = Caps(newTask: true, interject: true, watch: true)
        await h.mac.allow()
        await eventually("done") {
            if case .done = await h.state().pairing?.stage { return true }
            return false
        }

        // Keys and broker settings are stored; the Mac got `pair.done` with the phone's name.
        let record = try XCTUnwrap(h.vault.pairing(macId: h.mac.macId))
        XCTAssertEqual(record.macName, "Home Mac")
        XCTAssertEqual(record.phoneId, h.vault.phoneId())
        XCTAssertEqual(h.vault.broker?.username, "u1")
        await eventually("pair.done at the Mac") { h.mac.count(Wire.T.pairDone) == 1 }
        XCTAssertEqual(try h.mac.received(Wire.T.pairDone)[0].body(as: PairDone.self).name, "Test iPhone")

        // The session starts by itself.
        await eventually("online") { await h.state().macs.first?.link == .online }
        let state = await h.state()
        XCTAssertEqual(state.macs.first?.caps, Caps(newTask: true, interject: true, watch: true))
        XCTAssertEqual(state.macs.first?.slot, SlotInfo(active: true, autoActivation: false))
        XCTAssertEqual(h.events.contains(.pairingFinished(macId: h.mac.macId)), true)
    }

    func testPairDoneCarriesThePushHandleWhenTheTokenIsKnown() async throws {
        let h = Harness()
        await h.mac.start(password: "pw")
        await h.engine.start()
        await h.engine.updatePushToken(Data(repeating: 0xab, count: 32))
        let code = h.mac.startPairing(relayUrl: "https://relay.test", password: "pw")
        await h.engine.startPairing(code: code)
        await eventually("knock") { h.mac.sas != nil }
        await h.mac.allow()
        await eventually("pair.done") { h.mac.count(Wire.T.pairDone) == 1 }
        let done = try h.mac.received(Wire.T.pairDone)[0].body(as: PairDone.self)
        XCTAssertEqual(done.handle, "v1.handle-1")
        XCTAssertEqual(done.env, "dev")
        XCTAssertEqual(done.expiresAt, 1_900_000_000)
        XCTAssertEqual(h.vault.push?.handle, "v1.handle-1")
    }

    func testRejectionLeavesNothingBehind() async throws {
        let h = Harness()
        await h.mac.start(password: "pw")
        await h.engine.start()
        await h.engine.startPairing(code: h.mac.startPairing(password: "pw"))
        await eventually("knock") { h.mac.sas != nil }
        await h.mac.reject("rejected")
        await eventually("failure") { await h.state().pairing?.stage == .failed(.rejected("rejected")) }
        XCTAssertTrue(h.vault.pairings().isEmpty)
        XCTAssertNil(h.vault.broker)
        await h.engine.finishPairing()
        let cleared = await h.state().pairing
        XCTAssertNil(cleared)
    }

    func testBadAndExpiredCodesFailWithoutConnecting() async {
        let h = Harness()
        await h.mac.start(password: "pw")
        await h.engine.start()
        await h.engine.startPairing(code: "not a code")
        var stage = await h.state().pairing?.stage
        XCTAssertEqual(stage, .failed(.invalidCode))

        await h.engine.startPairing(code: h.mac.startPairing(password: "pw", expiresIn: -10))
        stage = await h.state().pairing?.stage
        XCTAssertEqual(stage, .failed(.expired))
        let phase = await h.state().phase
        XCTAssertEqual(phase, .idle, "nothing was connected")
    }

    func testWrongBrokerPasswordIsReportedAsAuthFailure() async {
        let h = Harness()
        await h.mac.start(password: "pw")
        await h.engine.start()
        await h.engine.startPairing(code: h.mac.startPairing(password: "wrong"))
        await eventually("auth failure") { await h.state().pairing?.stage == .failed(.authFailed) }
        let phase = await h.state().phase
        if case .refused = phase {} else { XCTFail("expected refused, got \(phase)") }
    }

    func testPairingTimesOutWhenTheTicketExpires() async {
        let h = Harness()
        await h.mac.start(password: "pw")
        await h.engine.start()
        await h.engine.startPairing(code: h.mac.startPairing(password: "pw", expiresIn: 60))
        await eventually("the Mac to react") { await h.state().pairing?.stage == .macConfirming }
        await h.clock.advance(by: 61)
        await eventually("expiry") { await h.state().pairing?.stage == .failed(.expired) }
    }

    func testAnotherBrokerAccountIsRefusedOncePaired() async throws {
        let h = Harness()
        await h.pair()
        let other = TestMac(broker: h.broker, user: "u2", clock: h.clock)
        await other.start(password: "pw2")
        await h.engine.startPairing(code: other.startPairing(password: "pw2"))
        let stage = await h.state().pairing?.stage
        XCTAssertEqual(stage, .failed(.differentBroker))
        XCTAssertEqual(h.vault.pairings().count, 1)
        other.stop()
    }

    func testPairingASecondMacOnTheSameBroker() async throws {
        let h = Harness()
        await h.pair()
        let second = TestMac(broker: h.broker, user: "u1", clock: h.clock, macName: "Office Mac")
        await second.start(password: "pw")
        await h.engine.startPairing(code: second.startPairing(password: "pw"))
        await eventually("knock") { second.sas != nil }
        await second.allow()
        await eventually("two Macs online") {
            let macs = await h.state().macs
            return macs.count == 2 && macs.allSatisfy { $0.link == .online }
        }
        XCTAssertEqual(Set(h.vault.pairings().map(\.macName)), ["Home Mac", "Office Mac"])
        second.stop()
    }

    func testPairingsSurviveARestart() async throws {
        let h = Harness()
        await h.pair()
        // A new engine over the same vault and store knows the Mac without pairing again.
        let phone2 = h.broker.makeClient()
        let engine2 = PhoneEngine(
            vault: h.vault, store: h.store, broker: phone2, clock: h.clock, relay: FakeRelay(),
            config: EngineConfig(deviceName: "Test iPhone", appVersion: "1.0", bundleId: "b", pushEnv: "dev", allowPlainLoopback: false, assetDirectory: h.root.appendingPathComponent("assets"))
        )
        await h.engine.setForeground(false)
        await engine2.start()
        await engine2.setForeground(true)
        await eventually("session without pairing") { await engine2.snapshot().macs.first?.link == .online }
    }
}

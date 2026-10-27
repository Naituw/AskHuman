import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

final class PushTests: XCTestCase {
    private let token = Data(repeating: 0xcd, count: 32)

    func testTheTokenIsRegisteredAtTheRelayAndTheHandleReachesTheMac() async throws {
        let h = Harness()
        await h.pair(relayUrl: "https://relay.test")
        await h.engine.updatePushToken(token)
        await eventually("push.register") { h.mac.count(Wire.T.pushRegister) == 1 }
        let register = try h.mac.received(Wire.T.pushRegister)[0].body(as: PushRegister.self)
        XCTAssertEqual(register.handle, "v1.handle-1")
        XCTAssertEqual(register.env, "dev")
        XCTAssertEqual(register.expiresAt, 1_900_000_000)
        XCTAssertEqual(h.relay.calls, 1)
        XCTAssertEqual(h.vault.push?.handle, "v1.handle-1")
        let status = await h.state().push
        XCTAssertEqual(status, .registered(expiresAt: Date(timeIntervalSince1970: 1_900_000_000)))
    }

    /// Found on a simulator: the permission alert makes the app inactive (the engine says goodbye) and
    /// active again at once; the token arrives in between, the handle was "sent" into a link that was
    /// going down, and the phone never tried again, so the Mac held no handle and no push ever came.
    func testAHandleThatCouldNotBeSentGoesOutWithTheNextHandshake() async throws {
        let h = Harness()
        await h.pair(relayUrl: "https://relay.test")
        h.phone.failsPublishes = true
        await h.engine.updatePushToken(token)
        XCTAssertEqual(h.relay.calls, 1)
        XCTAssertEqual(h.mac.count(Wire.T.pushRegister), 0, "nothing could be sent yet")

        h.phone.failsPublishes = false
        await h.engine.setForeground(false)
        await h.engine.setForeground(true)
        await eventually("push.register with the next handshake") { h.mac.count(Wire.T.pushRegister) == 1 }
        XCTAssertEqual(h.relay.calls, 1, "the handle is still the one the relay gave")
    }

    /// The token (and the relay's answer) can land between the Mac's ack and its first snapshot.
    func testAHandleThatIsReadyBetweenTheAckAndTheFirstSnapshotIsSentAtOnce() async throws {
        let h = Harness()
        await h.pair(relayUrl: "https://relay.test")
        await h.engine.setForeground(false)
        h.mac.holdSnapshots = true
        await h.engine.setForeground(true)
        let id = h.mac.macId
        await eventually("the handshake without snapshots") { await h.engine.macs[id]?.sync == .acked }

        await h.engine.updatePushToken(token)
        await eventually("push.register before any snapshot") { h.mac.count(Wire.T.pushRegister) == 1 }
        h.mac.holdSnapshots = false
        await h.mac.sendSnapshots(to: h.mac.pairedPhones[0])
        await eventually("the session to finish") { await h.state().macs.first?.link == .online }
        XCTAssertEqual(h.mac.count(Wire.T.pushRegister), 1)
    }

    /// The Mac never says that it lost a handle, so every session starts by giving it the current one.
    func testEveryHandshakeGivesTheMacTheHandleAgain() async throws {
        let h = Harness()
        await h.pair(relayUrl: "https://relay.test")
        await h.engine.updatePushToken(token)
        await eventually("first push.register") { h.mac.count(Wire.T.pushRegister) == 1 }

        await h.engine.setForeground(false)
        await h.engine.setForeground(true)
        await eventually("push.register of the second session") { h.mac.count(Wire.T.pushRegister) == 2 }
        XCTAssertEqual(h.relay.calls, 1, "the relay is not asked again")
        let handles = try h.mac.received(Wire.T.pushRegister).map { try $0.body(as: PushRegister.self).handle }
        XCTAssertEqual(handles, ["v1.handle-1", "v1.handle-1"])
    }

    func testTheSameTokenIsNotRegisteredTwice() async throws {
        let h = Harness()
        await h.pair(relayUrl: "https://relay.test")
        await h.engine.updatePushToken(token)
        await h.engine.updatePushToken(token)
        await eventually("push.register") { h.mac.count(Wire.T.pushRegister) >= 1 }
        try await Task.sleep(nanoseconds: 50_000_000)
        XCTAssertEqual(h.relay.calls, 1)
        XCTAssertEqual(h.mac.count(Wire.T.pushRegister), 1)
    }

    func testANewTokenGetsANewHandle() async throws {
        let h = Harness()
        await h.pair(relayUrl: "https://relay.test")
        await h.engine.updatePushToken(token)
        await h.engine.updatePushToken(Data(repeating: 0xee, count: 32))
        await eventually("two registrations") { h.mac.count(Wire.T.pushRegister) == 2 }
        XCTAssertEqual(h.relay.calls, 2)
        let handles = try h.mac.received(Wire.T.pushRegister).map { try $0.body(as: PushRegister.self).handle }
        XCTAssertEqual(handles, ["v1.handle-1", "v1.handle-2"])
    }

    func testNoRelayConfiguredMeansNoPush() async throws {
        let h = Harness()
        await h.pair(relayUrl: "")
        await h.engine.updatePushToken(token)
        let status = await h.state().push
        XCTAssertEqual(status, .noRelay)
        XCTAssertEqual(h.relay.calls, 0)
        XCTAssertEqual(h.mac.count(Wire.T.pushRegister), 0)
    }

    func testRelayFailureIsReportedAndRetriedWithTheNextToken() async throws {
        let h = Harness()
        await h.pair(relayUrl: "https://relay.test")
        h.relay.fail(with: TestFailure(message: "relay unreachable"))
        await h.engine.updatePushToken(token)
        let status = await h.state().push
        XCTAssertEqual(status, .failed("relay unreachable"))
        XCTAssertEqual(h.mac.count(Wire.T.pushRegister), 0)

        h.relay.fail(with: nil)
        await h.engine.updatePushToken(token)
        await eventually("push.register after the relay recovered") { h.mac.count(Wire.T.pushRegister) == 1 }
    }

    func testAHandleThatIsAboutToExpireIsRenewed() async throws {
        let h = Harness()
        // The handle expires within the renewal window.
        h.relay.expiresAt = Int64(h.clock.now().timeIntervalSince1970) + 10 * 24 * 3600
        await h.pair(relayUrl: "https://relay.test")
        await h.engine.updatePushToken(token)
        await h.engine.updatePushToken(token)
        XCTAssertEqual(h.relay.calls, 2, "a handle with less than 30 days left is renewed")
    }

    func testTheMacReportingAGoneHandleMakesThePhoneRenewIt() async throws {
        let h = Harness()
        await h.pair(relayUrl: "https://relay.test")
        await h.engine.updatePushToken(token)
        await eventually("first register") { h.mac.count(Wire.T.pushRegister) == 1 }
        await h.mac.sendDown(PushState(status: "gone"), to: h.mac.pairedPhones[0])
        await eventually("renewed and resent") { h.mac.count(Wire.T.pushRegister) == 2 }
        XCTAssertEqual(h.relay.calls, 2)
        let handle = try h.mac.received(Wire.T.pushRegister)[1].body(as: PushRegister.self).handle
        XCTAssertEqual(handle, "v1.handle-2")
    }

    func testTheMacMissingTheHandleGetsItAgainWithoutANewRegistration() async throws {
        let h = Harness()
        await h.pair(relayUrl: "https://relay.test")
        await h.engine.updatePushToken(token)
        await eventually("first register") { h.mac.count(Wire.T.pushRegister) == 1 }
        await h.mac.sendDown(PushState(status: "missing"), to: h.mac.pairedPhones[0])
        await eventually("resent") { h.mac.count(Wire.T.pushRegister) == 2 }
        XCTAssertEqual(h.relay.calls, 1)
        let handle = try h.mac.received(Wire.T.pushRegister)[1].body(as: PushRegister.self).handle
        XCTAssertEqual(handle, "v1.handle-1")
    }

    func testPushStatusOfTheMacIsShown() async throws {
        let h = Harness()
        await h.pair(relayUrl: "https://relay.test")
        await h.mac.sendDown(PushState(status: "ok"), to: h.mac.pairedPhones[0])
        await eventually("status") { await h.state().macs.first?.pushStatus == "ok" }
    }
}

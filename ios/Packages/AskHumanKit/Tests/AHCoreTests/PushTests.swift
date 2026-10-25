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
